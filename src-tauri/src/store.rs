use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use crate::provider::Provider;
use serde::{Deserialize, Serialize};

/// 配置文件 schema 版本（见 ADR 0001）。
pub const CONFIG_VERSION: u32 = 1;

/// `~/.maestro/config.json` 的顶层文档（version 1 schema）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    #[serde(default)]
    pub providers: BTreeMap<String, Provider>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            providers: BTreeMap::new(),
        }
    }
}

/// 默认配置路径：`~/.maestro/config.json`（见 ADR 0001）。
pub fn default_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "无法确定用户主目录（HOME）".to_owned())?;
    Ok(home.join(".maestro").join("config.json"))
}

/// 配置存储的错误；`message()` 面向最终用户。
#[derive(Debug, Clone)]
pub enum StoreError {
    /// 配置文件存在但无法解析。
    Corrupt { path: PathBuf, detail: String },
    /// 配置文件版本不被当前应用支持。
    UnsupportedVersion { path: PathBuf, found: u32 },
    /// 读写配置文件时发生 IO 错误。
    Io { path: PathBuf, detail: String },
    /// 已存在同名 Provider。
    DuplicateSlug { slug: String },
    /// Provider 不存在（update/delete 不做 upsert，绝不静默覆盖）。
    MissingSlug { slug: String },
}

impl StoreError {
    pub fn message(&self) -> String {
        match self {
            StoreError::Corrupt { path, detail } => format!(
                "配置文件已损坏：{}\n原因：{detail}\n请修复或删除该文件后重启应用；在此之前 Maestro 拒绝任何写入，绝不会静默重建。",
                path.display()
            ),
            StoreError::UnsupportedVersion { path, found } => format!(
                "配置文件版本不受支持：{}\n文件中的 version 为 {found}，当前应用仅支持 {CONFIG_VERSION}。\n请修复该文件后重启应用；在此之前 Maestro 拒绝任何写入，绝不会静默重建。",
                path.display()
            ),
            StoreError::Io { path, detail } if path.as_os_str().is_empty() => {
                format!("读写配置文件失败\n原因：{detail}")
            }
            StoreError::Io { path, detail } => {
                format!("读写配置文件失败：{}\n原因：{detail}", path.display())
            }
            StoreError::DuplicateSlug { slug } => format!("已存在同名 Provider：{slug}"),
            StoreError::MissingSlug { slug } => format!("Provider 不存在：{slug}"),
        }
    }
}

/// 配置存储：启动时从磁盘加载进内存，变更后原子写回。
///
/// 配置文件损坏（无法解析或版本不受支持）时进入保护状态：
/// 读取与写入一律报错，绝不静默重建或覆盖原文件。
pub struct Store {
    path: PathBuf,
    state: Result<Config, StoreError>,
}

impl Store {
    /// 从磁盘加载配置。文件不存在视为首次使用（空配置）；损坏则进入保护状态。
    pub fn open(path: PathBuf) -> Self {
        let state = match fs::read_to_string(&path) {
            Ok(text) => Self::parse(&path, &text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(StoreError::Io {
                path: path.clone(),
                detail: e.to_string(),
            }),
        };
        Self { path, state }
    }

    /// 构造不可用状态的存储（例如无法确定主目录时）；读取与写入一律报错。
    pub fn unavailable(detail: String) -> Self {
        Self {
            path: PathBuf::new(),
            state: Err(StoreError::Io {
                path: PathBuf::new(),
                detail,
            }),
        }
    }

    fn parse(path: &Path, text: &str) -> Result<Config, StoreError> {
        let config: Config = serde_json::from_str(text).map_err(|e| StoreError::Corrupt {
            path: path.to_owned(),
            detail: e.to_string(),
        })?;
        if config.version != CONFIG_VERSION {
            return Err(StoreError::UnsupportedVersion {
                path: path.to_owned(),
                found: config.version,
            });
        }
        Ok(config)
    }

    /// 当前配置；存储处于保护状态时返回错误。
    pub fn get(&self) -> Result<&Config, &StoreError> {
        self.state.as_ref()
    }

    /// 新建一条 Provider（slug 唯一），成功后原子写回磁盘。
    pub fn create_provider(&mut self, slug: &str, provider: Provider) -> Result<(), StoreError> {
        let config = self.state.as_ref().map_err(Clone::clone)?;
        if config.providers.contains_key(slug) {
            return Err(StoreError::DuplicateSlug {
                slug: slug.to_owned(),
            });
        }
        let mut next = config.clone();
        next.providers.insert(slug.to_owned(), provider);
        self.persist(&next)?;
        self.state = Ok(next);
        Ok(())
    }

    /// 整包替换指定 Provider 的端点与模型列表；api_key 一律保持原值不变
    /// （凭证的三态更新契约见 ADR 0002，不经本方法实现）。
    /// slug 不存在时报错，不做 upsert。
    pub fn update_provider(&mut self, slug: &str, provider: Provider) -> Result<(), StoreError> {
        let config = self.state.as_ref().map_err(Clone::clone)?;
        let mut next = config.clone();
        let Some(target) = next.providers.get_mut(slug) else {
            return Err(StoreError::MissingSlug {
                slug: slug.to_owned(),
            });
        };
        let api_key = target.api_key.clone();
        *target = Provider {
            api_key,
            ..provider
        };
        self.persist(&next)?;
        self.state = Ok(next);
        Ok(())
    }

    /// 删除 Provider（其模型数据随记录一并移除），并返回删除前的密钥引用，交由上层清除。
    ///
    /// 本方法不触碰密钥链：返回值中 `Some` 为需要清除的密钥引用，`None` 表示未设置
    /// 凭证、无需清除；密钥链清除失败不得阻塞删除，由上层降级为警告（见命令层）。
    pub fn delete_provider(&mut self, slug: &str) -> Result<Vec<Option<String>>, StoreError> {
        let config = self.state.as_ref().map_err(Clone::clone)?;
        let mut next = config.clone();
        let Some(provider) = next.providers.get(slug) else {
            return Err(StoreError::MissingSlug {
                slug: slug.to_owned(),
            });
        };

        let secret_references = vec![provider.api_key.clone()];

        next.providers.remove(slug);
        self.persist(&next)?;
        self.state = Ok(next);

        Ok(secret_references)
    }

    /// 原子写入：先写同目录临时文件并落盘，再 rename 覆盖目标，避免半截文件。
    fn persist(&self, config: &Config) -> Result<(), StoreError> {
        let dir = self.path.parent().unwrap_or_else(|| Path::new("."));
        let io_error = |e: std::io::Error| StoreError::Io {
            path: self.path.clone(),
            detail: e.to_string(),
        };
        fs::create_dir_all(dir).map_err(io_error)?;
        let json = serde_json::to_string_pretty(config).map_err(|e| StoreError::Io {
            path: self.path.clone(),
            detail: e.to_string(),
        })?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(io_error)?;
        tmp.write_all(format!("{json}\n").as_bytes())
            .map_err(io_error)?;
        tmp.as_file().sync_all().map_err(io_error)?;
        // PersistError 内含真正的 io::Error；临时文件随后 drop 时自动清理。
        tmp.persist(&self.path).map_err(|e| io_error(e.error))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{Endpoints, ModelEntry};

    #[test]
    fn unavailable_store_refuses_reads_and_writes() {
        let mut store = Store::unavailable("无法确定用户主目录（HOME）".to_owned());

        let err = store.get().unwrap_err();
        assert!(err.message().contains("无法确定用户主目录"));
        assert!(store
            .create_provider(
                "foo",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:9".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: None,
                    models: Vec::new(),
                }
            )
            .is_err());
        assert!(store
            .update_provider(
                "foo",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:9".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: None,
                    models: Vec::new(),
                }
            )
            .is_err());
        assert!(store.delete_provider("foo").is_err());
    }

    #[test]
    fn empty_config_serializes_to_version_1_schema() {
        let config = Config::default();

        let text = serde_json::to_string(&config).unwrap();

        assert_eq!(text, r#"{"version":1,"providers":{}}"#);
        let parsed: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn config_parses_version_1_document() {
        let text = r#"{
            "version": 1,
            "providers": {
                "ollama": {
                    "base_url": {
                        "openai-completions": "http://localhost:11434/v1"
                    },
                    "models": [
                        { "id": "deepseek-chat", "display_name": "DeepSeek Chat" },
                        { "id": "deepseek-reasoner", "display_name": null }
                    ]
                }
            }
        }"#;

        let parsed: Config = serde_json::from_str(text).unwrap();

        let provider = &parsed.providers["ollama"];
        assert_eq!(
            provider.base_url.openai_completions,
            Some("http://localhost:11434/v1".to_owned())
        );
        assert_eq!(provider.base_url.anthropic_messages, None);
        assert_eq!(provider.api_key, None);
        assert_eq!(provider.models.len(), 2);
        assert_eq!(provider.models[0].id, "deepseek-chat");
        assert_eq!(
            provider.models[0].display_name.as_deref(),
            Some("DeepSeek Chat")
        );
        assert_eq!(provider.models[1].id, "deepseek-reasoner");
        assert_eq!(provider.models[1].display_name, None);
    }

    #[test]
    fn missing_file_opens_as_empty_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        let store = Store::open(path);

        assert_eq!(store.get().unwrap(), &Config::default());
        assert!(store.get().unwrap().providers.is_empty());
    }

    #[test]
    fn create_provider_persists_and_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut store = Store::open(path.clone());

        store
            .create_provider(
                "ollama",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:11434/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: None,
                    models: Vec::new(),
                },
            )
            .unwrap();

        let reopened = Store::open(path);
        let provider = &reopened.get().unwrap().providers["ollama"];
        assert_eq!(
            provider.base_url.openai_completions,
            Some("http://localhost:11434/v1".to_owned())
        );
        assert_eq!(provider.base_url.anthropic_messages, None);
        assert_eq!(provider.api_key, None);
        assert!(provider.models.is_empty());
    }

    #[test]
    fn update_provider_persists_and_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut store = Store::open(path.clone());
        store
            .create_provider(
                "ollama",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:11434/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: None,
                    models: Vec::new(),
                },
            )
            .unwrap();

        store
            .update_provider(
                "ollama",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("https://api.example.com/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: None,
                    models: Vec::new(),
                },
            )
            .unwrap();

        let reopened = Store::open(path);
        let provider = &reopened.get().unwrap().providers["ollama"];
        assert_eq!(
            provider.base_url.openai_completions,
            Some("https://api.example.com/v1".to_owned())
        );
        assert_eq!(provider.base_url.anthropic_messages, None);
        assert_eq!(provider.api_key, None);
        assert!(provider.models.is_empty());
    }

    #[test]
    fn update_provider_missing_slug_is_rejected_without_upsert() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut store = Store::open(path.clone());

        let err = store
            .update_provider(
                "ghost",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:9".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: None,
                    models: Vec::new(),
                },
            )
            .unwrap_err();
        assert!(matches!(err, StoreError::MissingSlug { .. }));
        assert!(err.message().contains("ghost"));
        assert!(store.get().unwrap().providers.is_empty());
        assert!(!path.exists(), "报错路径不得静默写入文件");
    }

    #[test]
    fn update_provider_keeps_api_key_and_replaces_endpoints_and_models() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut store = Store::open(path.clone());
        let reference = "secret://io.github.xezzon.agent-maestro/provider/ollama/api_key";
        store
            .create_provider(
                "ollama",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:11434/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: Option::Some(reference.to_owned()),
                    models: vec![ModelEntry {
                        id: "old-model".to_owned(),
                        display_name: Option::None,
                    }],
                },
            )
            .unwrap();

        store
            .update_provider(
                "ollama",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("https://api.example.com/v1".to_owned()),
                        anthropic_messages: Option::Some("http://127.0.0.1:8080".to_owned()),
                    },
                    api_key: Option::Some(
                        "secret://io.github.xezzon.agent-maestro/provider/other/api_key".to_owned(),
                    ),
                    models: vec![ModelEntry {
                        id: "new-model".to_owned(),
                        display_name: Option::None,
                    }],
                },
            )
            .unwrap();

        let provider = &store.get().unwrap().providers["ollama"];
        assert_eq!(
            provider.api_key.as_deref(),
            Some(reference),
            "api_key 不经 update_provider 变更，请求中携带的密钥负载被忽略"
        );
        assert_eq!(
            provider.base_url.openai_completions.as_deref(),
            Some("https://api.example.com/v1")
        );
        assert_eq!(
            provider.base_url.anthropic_messages.as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(provider.models.len(), 1);
        assert_eq!(provider.models[0].id, "new-model", "模型列表整包替换");

        let reopened = Store::open(path);
        let provider = &reopened.get().unwrap().providers["ollama"];
        assert_eq!(provider.api_key.as_deref(), Some(reference));
        assert_eq!(provider.models[0].id, "new-model");
    }

    #[test]
    fn delete_provider_returns_secret_reference_and_removes_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(
            &path,
            r#"{
                "version": 1,
                "providers": {
                    "openrouter": {
                        "base_url": {
                            "openai-completions": "https://api.example.com/v1"
                        },
                        "api_key": "secret://io.github.xezzon.agent-maestro/provider/openrouter/api_key",
                        "models": [
                            { "id": "z-model", "display_name": null },
                            { "id": "a-model", "display_name": "A Model" }
                        ]
                    }
                }
            }"#,
        )
        .unwrap();
        let mut store = Store::open(path.clone());

        let secret_references = store.delete_provider("openrouter").unwrap();

        assert_eq!(
            secret_references,
            vec![Some(
                "secret://io.github.xezzon.agent-maestro/provider/openrouter/api_key".to_owned()
            )],
            "删除后必须原样返回密钥引用，供上层清除密钥链条目"
        );
        let reopened = Store::open(path);
        assert!(reopened.get().unwrap().providers.is_empty());
    }

    #[test]
    fn delete_provider_without_api_key_returns_no_reference() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut store = Store::open(path.clone());
        store
            .create_provider(
                "ollama",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:11434/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: None,
                    models: Vec::new(),
                },
            )
            .unwrap();

        let secret_references = store.delete_provider("ollama").unwrap();

        assert_eq!(
            secret_references,
            vec![None],
            "未设置凭证时返回 None，上层无需清除密钥链"
        );
        let reopened = Store::open(path);
        assert!(reopened.get().unwrap().providers.is_empty());
    }

    #[test]
    fn delete_provider_missing_slug_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut store = Store::open(path);

        let err = store.delete_provider("ghost").unwrap_err();

        assert!(matches!(err, StoreError::MissingSlug { .. }));
        assert!(err.message().contains("ghost"));
    }

    #[test]
    fn deleted_slug_can_be_recreated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut store = Store::open(path.clone());
        store
            .create_provider(
                "ollama",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:11434/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: None,
                    models: Vec::new(),
                },
            )
            .unwrap();

        store.delete_provider("ollama").unwrap();
        store
            .create_provider(
                "ollama",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::None,
                        anthropic_messages: Option::Some("http://127.0.0.1:8080".to_owned()),
                    },
                    api_key: None,
                    models: Vec::new(),
                },
            )
            .unwrap();

        let reopened = Store::open(path);
        let provider = &reopened.get().unwrap().providers["ollama"];
        assert_eq!(
            provider.base_url.anthropic_messages,
            Some("http://127.0.0.1:8080".to_owned())
        );
        assert_eq!(provider.base_url.openai_completions, None);
        assert!(provider.models.is_empty());
    }

    #[test]
    fn persisted_file_omits_unconfigured_protocol_slots() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut store = Store::open(path.clone());

        store
            .create_provider(
                "ollama",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:11434/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: None,
                    models: Vec::new(),
                },
            )
            .unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#""openai-completions""#));
        assert!(
            !text.contains(r#""anthropic-messages""#),
            "未配置的协议槽不得写入文件（ADR 0003：键缺省而非空串）"
        );
    }

    #[test]
    fn providers_are_written_sorted_by_slug() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut store = Store::open(path.clone());

        for slug in ["zeta", "alpha", "midway"] {
            store
                .create_provider(
                    slug,
                    Provider {
                        base_url: Endpoints {
                            openai_completions: Option::Some("http://localhost:9/v1".to_owned()),
                            anthropic_messages: Option::None,
                        },
                        api_key: None,
                        models: Vec::new(),
                    },
                )
                .unwrap();
        }

        let text = fs::read_to_string(&path).unwrap();
        let alpha = text.find("\"alpha\"").unwrap();
        let midway = text.find("\"midway\"").unwrap();
        let zeta = text.find("\"zeta\"").unwrap();
        assert!(alpha < midway && midway < zeta);
    }

    #[test]
    fn multiple_providers_round_trip_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut store = Store::open(path.clone());

        store
            .create_provider(
                "ollama",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:11434/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: None,
                    models: Vec::new(),
                },
            )
            .unwrap();
        store
            .create_provider(
                "openrouter",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::None,
                        anthropic_messages: Option::Some(
                            "https://anthropic.example.com/v1".to_owned(),
                        ),
                    },
                    api_key: None,
                    models: Vec::new(),
                },
            )
            .unwrap();

        let reopened = Store::open(path);
        let config = reopened.get().unwrap();
        assert_eq!(config.providers.len(), 2);
        assert_eq!(
            config.providers["ollama"].base_url.openai_completions,
            Some("http://localhost:11434/v1".to_owned())
        );
        assert_eq!(
            config.providers["openrouter"].base_url.anthropic_messages,
            Some("https://anthropic.example.com/v1".to_owned())
        );
    }

    #[test]
    fn create_provider_preserves_other_providers_models_and_endpoints() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(
            &path,
            r#"{
                "version": 1,
                "providers": {
                    "openrouter": {
                        "base_url": {
                            "openai-completions": "https://api.example.com/v1",
                            "anthropic-messages": "https://anthropic.example.com/v1"
                        },
                        "api_key": "secret://io.github.xezzon.agent-maestro/provider/openrouter/api_key",
                        "models": [
                            { "id": "z-model", "display_name": null },
                            { "id": "a-model", "display_name": "A Model" }
                        ]
                    }
                }
            }"#,
        )
        .unwrap();
        let mut store = Store::open(path.clone());

        store
            .create_provider(
                "ollama",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:11434/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: None,
                    models: Vec::new(),
                },
            )
            .unwrap();

        let reopened = Store::open(path);
        let openrouter = &reopened.get().unwrap().providers["openrouter"];
        assert_eq!(
            openrouter.api_key,
            Some("secret://io.github.xezzon.agent-maestro/provider/openrouter/api_key".to_owned())
        );
        assert_eq!(openrouter.models.len(), 2);
        assert_eq!(openrouter.models[0].id, "z-model");
        assert_eq!(openrouter.models[1].id, "a-model");
        assert_eq!(
            openrouter.base_url.openai_completions,
            Some("https://api.example.com/v1".to_owned())
        );
        assert_eq!(
            openrouter.base_url.anthropic_messages,
            Some("https://anthropic.example.com/v1".to_owned())
        );
    }

    #[test]
    fn corrupt_file_reports_error_with_path_and_refuses_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, "不是 JSON {{{").unwrap();
        let original = fs::read_to_string(&path).unwrap();

        let mut store = Store::open(path.clone());

        let err = store.get().unwrap_err();
        assert!(matches!(err, StoreError::Corrupt { .. }));
        assert!(err.message().contains(path.to_str().unwrap()));

        assert!(store
            .create_provider(
                "foo",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:9".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: None,
                    models: Vec::new(),
                }
            )
            .is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn unsupported_version_reports_error_with_path_and_refuses_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, r#"{"version":99,"providers":{}}"#).unwrap();
        let original = fs::read_to_string(&path).unwrap();

        let mut store = Store::open(path.clone());

        let err = store.get().unwrap_err();
        assert!(matches!(err, StoreError::UnsupportedVersion { .. }));
        assert!(err.message().contains(path.to_str().unwrap()));

        assert!(store
            .create_provider(
                "foo",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:9".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: None,
                    models: Vec::new(),
                }
            )
            .is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn duplicate_slug_is_rejected_and_original_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut store = Store::open(path);

        store
            .create_provider(
                "foo",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:9/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: None,
                    models: Vec::new(),
                },
            )
            .unwrap();

        let err = store
            .create_provider(
                "foo",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::None,
                        anthropic_messages: Option::Some("http://localhost:10".to_owned()),
                    },
                    api_key: None,
                    models: Vec::new(),
                },
            )
            .unwrap_err();
        assert!(matches!(err, StoreError::DuplicateSlug { .. }));

        let config = store.get().unwrap();
        assert_eq!(config.providers.len(), 1);
        assert_eq!(
            config.providers["foo"].base_url.openai_completions,
            Some("http://localhost:9/v1".to_owned())
        );
        assert_eq!(config.providers["foo"].base_url.anthropic_messages, None);
    }

    #[test]
    fn create_provider_creates_missing_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".maestro/config.json");
        let mut store = Store::open(path.clone());

        store
            .create_provider(
                "foo",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::None,
                        anthropic_messages: Option::Some("http://127.0.0.1:8080".to_owned()),
                    },
                    api_key: None,
                    models: Vec::new(),
                },
            )
            .unwrap();

        assert!(path.exists());
    }
}
