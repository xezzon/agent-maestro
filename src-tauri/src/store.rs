use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

use crate::{paths::MaestroPaths, plugin::PluginEntry, provider::Provider};
use serde::{Deserialize, Serialize};

/// 配置文件 schema 版本（见 ADR 0001）。
pub const CONFIG_VERSION: u32 = 1;

/// 配置存储的锁被毒化（其它命令持锁期间 panic）时对用户可见的原因。
/// `AppStore` 与插件服务共用同一句文案：两条加锁路径对用户是同一件事。
pub const STORE_LOCK_POISONED: &str = "配置存储不可用";

/// `~/.maestro/config.json` 的顶层文档（version 1 schema）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    #[serde(default)]
    pub providers: BTreeMap<String, Provider>,
    /// 无插件时省略该段，保持与旧配置文件一致。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<PluginEntry>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            providers: BTreeMap::new(),
            plugins: Vec::new(),
        }
    }
}

/// 配置存储的错误；转为 `String` 时面向最终用户。
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
    /// 插件条目不存在。
    MissingSource { source: String },
    /// 已存在同一来源的插件条目（来源即条目唯一身份）。
    DuplicateSource { source: String },
}

impl From<&StoreError> for String {
    fn from(value: &StoreError) -> Self {
        match value {
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
            StoreError::MissingSource { source } => format!("插件条目不存在：{source}"),
            StoreError::DuplicateSource { source } => {
                format!("已存在同一来源的插件条目：{source}")
            }
        }
    }
}

impl From<StoreError> for String {
    fn from(value: StoreError) -> Self {
        (&value).into()
    }
}

/// 配置存储：启动时从磁盘加载进内存，变更后原子写回。
///
/// 配置文件损坏（无法解析或版本不受支持）时进入保护状态：
/// 读取与写入一律报错，绝不静默重建或覆盖原文件。
pub struct Store {
    config_path: PathBuf,
    state: Result<Config, StoreError>,
}

impl Store {
    /// 从 `maestro_paths` 取配置路径并加载配置。文件不存在视为首次使用（空配置）；
    /// 损坏则进入保护状态。
    pub fn new(maestro_paths: &MaestroPaths) -> Self {
        let config_path = maestro_paths.config_path();
        let state = match fs::read_to_string(&config_path) {
            Ok(text) => Self::parse(&config_path, &text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(StoreError::Io {
                path: config_path.clone(),
                detail: e.to_string(),
            }),
        };
        Self { config_path, state }
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

    /// 整包替换指定 Provider 的端点、模型列表与 API Key（明文）。
    /// slug 不存在时报错，不做 upsert。
    pub fn update_provider(
        &mut self,
        slug: &str,
        provider: Provider,
    ) -> Result<Provider, StoreError> {
        let config = self.state.as_ref().map_err(Clone::clone)?;
        let mut next = config.clone();

        match next.providers.get(slug).cloned() {
            Some(original) => {
                next.providers.insert(slug.to_owned(), provider);
                self.persist(&next)?;
                self.state = Ok(next);
                Ok(original)
            }
            None => Err(StoreError::MissingSlug {
                slug: slug.to_owned(),
            }),
        }
    }

    /// 删除 Provider，其端点、模型与 API Key 随记录一并移除，不留孤儿数据。
    pub fn delete_provider(&mut self, slug: &str) -> Result<Provider, StoreError> {
        let config = self.state.as_ref().map_err(Clone::clone)?;
        let mut next = config.clone();

        match next.providers.remove(slug) {
            Some(original) => {
                self.persist(&next)?;
                self.state = Ok(next);
                Ok(original)
            }
            None => Err(StoreError::MissingSlug {
                slug: slug.to_owned(),
            }),
        }
    }

    /// 新增插件条目（`source` 即唯一身份，重复添加即拒绝），追加在现有条目之后。
    ///
    /// 安装成功后由插件服务调用，`id` 随条目一并给出：它是来源到落位目录的唯一映射
    /// （见 ADR 0006）。因此安装失败时不会留下条目，重新添加即可。
    pub fn add_plugin(&mut self, plugin_entry: &PluginEntry) -> Result<(), StoreError> {
        let source = plugin_entry.source.clone();
        let config = self.state.as_ref().map_err(Clone::clone)?;
        if config.plugins.iter().any(|plugin| plugin.source == source) {
            return Err(StoreError::DuplicateSource {
                source: source.to_owned(),
            });
        }
        let mut next = config.clone();
        next.plugins.push(plugin_entry.clone());
        self.persist(&next)?;
        self.state = Ok(next);

        Ok(())
    }

    /// 删除插件条目，返回被删除的条目（调用方据此删除落位目录）。
    pub fn delete_plugin(&mut self, source: &str) -> Result<PluginEntry, StoreError> {
        let config = self.state.as_ref().map_err(Clone::clone)?;
        let mut next = config.clone();
        let index = next
            .plugins
            .iter()
            .position(|plugin| plugin.source == source)
            .ok_or_else(|| StoreError::MissingSource {
                source: source.to_owned(),
            })?;
        let removed = next.plugins.remove(index);
        self.persist(&next)?;
        self.state = Ok(next);
        Ok(removed)
    }

    pub fn list_plugins(&self) -> Result<Vec<PluginEntry>, StoreError> {
        let config = self.state.as_ref().map_err(Clone::clone)?;
        Ok(config.plugins.clone())
    }

    /// 按来源取插件条目（`source` 即条目唯一身份）。
    pub fn plugin_by_source(&self, source: &str) -> Result<PluginEntry, StoreError> {
        let config = self.state.as_ref().map_err(Clone::clone)?;
        config
            .plugins
            .iter()
            .find(|plugin| plugin.source == source)
            .cloned()
            .ok_or_else(|| StoreError::MissingSource {
                source: source.to_owned(),
            })
    }

    /// 以 `source` 定位并原位修改 plugins 段；找不到即报错，绝不静默写入。
    fn update_plugins(
        &mut self,
        source: &str,
        f: impl FnOnce(&mut Vec<PluginEntry>) -> bool,
    ) -> Result<(), StoreError> {
        let config = self.state.as_ref().map_err(Clone::clone)?;
        let mut next = config.clone();
        if !f(&mut next.plugins) {
            return Err(StoreError::MissingSource {
                source: source.to_owned(),
            });
        }
        self.persist(&next)?;
        self.state = Ok(next);
        Ok(())
    }

    /// 启用/禁用插件条目。
    pub fn set_plugin_enabled(&mut self, source: &str, enabled: bool) -> Result<(), StoreError> {
        self.update_plugins(source, |plugins| {
            for plugin in plugins.iter_mut() {
                if plugin.source == source {
                    plugin.enabled = enabled;
                    return true;
                }
            }
            false
        })
    }

    /// 原子写入：先写同目录临时文件并落盘，再 rename 覆盖目标，避免半截文件。
    fn persist(&self, config: &Config) -> Result<(), StoreError> {
        let dir = self.config_path.parent().unwrap_or_else(|| Path::new("."));
        let io_error = |e: std::io::Error| StoreError::Io {
            path: self.config_path.clone(),
            detail: e.to_string(),
        };
        fs::create_dir_all(dir).map_err(io_error)?;

        let json = serde_json::to_string_pretty(config).map_err(|e| StoreError::Io {
            path: self.config_path.clone(),
            detail: e.to_string(),
        })?;

        let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(io_error)?;
        tmp.write_all(format!("{json}\n").as_bytes())
            .map_err(io_error)?;
        tmp.as_file().sync_all().map_err(io_error)?;

        // PersistError 内含真正的 io::Error；临时文件随后 drop 时自动清理。
        tmp.persist(&self.config_path)
            .map_err(|e| io_error(e.error))?;
        Ok(())
    }
}

/// 共享应用状态：配置存储（启动时加载进内存，变更后原子写回）。
pub struct AppStore {
    pub store: Mutex<Store>,
}

impl AppStore {
    /// 取配置存储的锁。其它命令持锁期间 panic 会毒化锁，
    /// 此时报错而非静默继续（读取或写入都不可信）。
    pub fn lock(&self) -> Result<std::sync::MutexGuard<'_, Store>, String> {
        self.store
            .lock()
            .map_err(|_| STORE_LOCK_POISONED.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{Endpoints, ModelEntry};

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
        assert_eq!(provider.api_key, "");
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
        let maestro_paths = MaestroPaths::new(dir.path());
        let store = Store::new(&maestro_paths);

        assert_eq!(store.get().unwrap(), &Config::default());
        assert!(store.get().unwrap().providers.is_empty());
    }

    #[test]
    fn create_provider_persists_and_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let maestro_paths = MaestroPaths::new(dir.path());
        let mut store = Store::new(&maestro_paths);

        store
            .create_provider(
                "ollama",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:11434/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: String::new(),
                    models: Vec::new(),
                },
            )
            .unwrap();

        let reopened = Store::new(&maestro_paths);
        let provider = &reopened.get().unwrap().providers["ollama"];
        assert_eq!(
            provider.base_url.openai_completions,
            Some("http://localhost:11434/v1".to_owned())
        );
        assert_eq!(provider.base_url.anthropic_messages, None);
        assert_eq!(provider.api_key, "");
        assert!(provider.models.is_empty());
    }

    #[test]
    fn update_provider_persists_and_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let maestro_paths = MaestroPaths::new(dir.path());
        let mut store = Store::new(&maestro_paths);
        store
            .create_provider(
                "ollama",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:11434/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: String::new(),
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
                    api_key: "sk-test".to_owned(),
                    models: Vec::new(),
                },
            )
            .unwrap();

        let reopened = Store::new(&maestro_paths);
        let provider = &reopened.get().unwrap().providers["ollama"];
        assert_eq!(
            provider.base_url.openai_completions,
            Some("https://api.example.com/v1".to_owned())
        );
        assert_eq!(provider.base_url.anthropic_messages, None);
        assert_eq!(provider.api_key, "sk-test");
        assert!(provider.models.is_empty());
    }

    #[test]
    fn update_provider_missing_slug_is_rejected_without_upsert() {
        let dir = tempfile::tempdir().unwrap();
        let maestro_paths = MaestroPaths::new(dir.path());
        let mut store = Store::new(&maestro_paths);

        let err = store
            .update_provider(
                "ghost",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:9".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: String::new(),
                    models: Vec::new(),
                },
            )
            .unwrap_err();
        assert!(matches!(err, StoreError::MissingSlug { .. }));
        assert!(String::from(err).contains("ghost"));
        assert!(store.get().unwrap().providers.is_empty());
        assert!(
            !maestro_paths.config_path().exists(),
            "报错路径不得静默写入文件"
        );
    }

    #[test]
    fn update_provider_replaces_api_key_with_whole_record() {
        let dir = tempfile::tempdir().unwrap();
        let maestro_paths = MaestroPaths::new(dir.path());
        let mut store = Store::new(&maestro_paths);
        store
            .create_provider(
                "ollama",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:11434/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: "sk-old".to_owned(),
                    models: vec![ModelEntry {
                        id: "old-model".to_owned(),
                        display_name: Option::None,
                    }],
                },
            )
            .unwrap();

        // 整包替换：api_key 携带新值即覆盖为明文。
        store
            .update_provider(
                "ollama",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("https://api.example.com/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: "sk-new".to_owned(),
                    models: vec![ModelEntry {
                        id: "new-model".to_owned(),
                        display_name: Option::None,
                    }],
                },
            )
            .unwrap();
        let provider = &store.get().unwrap().providers["ollama"];
        assert_eq!(provider.api_key, "sk-new", "api_key 随整包替换覆盖为明文");
        assert_eq!(
            provider.base_url.openai_completions.as_deref(),
            Some("https://api.example.com/v1"),
            "端点整包替换"
        );
        assert_eq!(provider.models[0].id, "new-model", "模型列表整包替换");

        // api_key 为空串即清除凭证。
        store
            .update_provider(
                "ollama",
                Provider {
                    api_key: String::new(),
                    ..provider.clone()
                },
            )
            .unwrap();
        assert_eq!(
            store.get().unwrap().providers["ollama"].api_key,
            "",
            "api_key 为空串即清除凭证"
        );

        let reopened = Store::new(&maestro_paths);
        assert_eq!(reopened.get().unwrap().providers["ollama"].api_key, "");
    }

    #[test]
    fn delete_provider_removes_record_with_models_and_api_key() {
        let dir = tempfile::tempdir().unwrap();
        let maestro_paths = MaestroPaths::new(dir.path());
        fs::create_dir_all(maestro_paths.maestro_dir()).unwrap();
        fs::write(
            maestro_paths.config_path(),
            r#"{
                "version": 1,
                "providers": {
                    "openrouter": {
                        "base_url": {
                            "openai-completions": "https://api.example.com/v1"
                        },
                        "api_key": "sk-live",
                        "models": [
                            { "id": "z-model", "display_name": null },
                            { "id": "a-model", "display_name": "A Model" }
                        ]
                    }
                }
            }"#,
        )
        .unwrap();
        let mut store = Store::new(&maestro_paths);

        store.delete_provider("openrouter").unwrap();

        let reopened = Store::new(&maestro_paths);
        assert!(
            reopened.get().unwrap().providers.is_empty(),
            "删除后不留孤儿数据：端点、模型与 API Key 随记录一并移除"
        );
    }

    #[test]
    fn delete_provider_missing_slug_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let maestro_paths = MaestroPaths::new(dir.path());
        let mut store = Store::new(&maestro_paths);

        let err = store.delete_provider("ghost").unwrap_err();

        assert!(matches!(err, StoreError::MissingSlug { .. }));
        assert!(String::from(err).contains("ghost"));
    }

    #[test]
    fn deleted_slug_can_be_recreated() {
        let dir = tempfile::tempdir().unwrap();
        let maestro_paths = MaestroPaths::new(dir.path());
        let mut store = Store::new(&maestro_paths);
        store
            .create_provider(
                "ollama",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:11434/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: String::new(),
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
                    api_key: String::new(),
                    models: Vec::new(),
                },
            )
            .unwrap();

        let reopened = Store::new(&maestro_paths);
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
        let maestro_paths = MaestroPaths::new(dir.path());
        let mut store = Store::new(&maestro_paths);

        store
            .create_provider(
                "ollama",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:11434/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: String::new(),
                    models: Vec::new(),
                },
            )
            .unwrap();

        let text = fs::read_to_string(maestro_paths.config_path()).unwrap();
        assert!(text.contains(r#""openai-completions""#));
        assert!(
            !text.contains(r#""anthropic-messages""#),
            "未配置的协议槽不得写入文件（ADR 0003：键缺省而非空串）"
        );
    }

    #[test]
    fn providers_are_written_sorted_by_slug() {
        let dir = tempfile::tempdir().unwrap();
        let maestro_paths = MaestroPaths::new(dir.path());
        let mut store = Store::new(&maestro_paths);

        for slug in ["zeta", "alpha", "midway"] {
            store
                .create_provider(
                    slug,
                    Provider {
                        base_url: Endpoints {
                            openai_completions: Option::Some("http://localhost:9/v1".to_owned()),
                            anthropic_messages: Option::None,
                        },
                        api_key: String::new(),
                        models: Vec::new(),
                    },
                )
                .unwrap();
        }

        let text = fs::read_to_string(maestro_paths.config_path()).unwrap();
        let alpha = text.find("\"alpha\"").unwrap();
        let midway = text.find("\"midway\"").unwrap();
        let zeta = text.find("\"zeta\"").unwrap();
        assert!(alpha < midway && midway < zeta);
    }

    #[test]
    fn multiple_providers_round_trip_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let maestro_paths = MaestroPaths::new(dir.path());
        let mut store = Store::new(&maestro_paths);

        store
            .create_provider(
                "ollama",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:11434/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: String::new(),
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
                    api_key: String::new(),
                    models: Vec::new(),
                },
            )
            .unwrap();

        let reopened = Store::new(&maestro_paths);
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
        let maestro_paths = MaestroPaths::new(dir.path());
        fs::create_dir_all(maestro_paths.maestro_dir()).unwrap();
        fs::write(
            maestro_paths.config_path(),
            r#"{
                "version": 1,
                "providers": {
                    "openrouter": {
                        "base_url": {
                            "openai-completions": "https://api.example.com/v1",
                            "anthropic-messages": "https://anthropic.example.com/v1"
                        },
                        "api_key": "sk-live",
                        "models": [
                            { "id": "z-model", "display_name": null },
                            { "id": "a-model", "display_name": "A Model" }
                        ]
                    }
                }
            }"#,
        )
        .unwrap();
        let mut store = Store::new(&maestro_paths);

        store
            .create_provider(
                "ollama",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:11434/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: String::new(),
                    models: Vec::new(),
                },
            )
            .unwrap();

        let reopened = Store::new(&maestro_paths);
        let openrouter = &reopened.get().unwrap().providers["openrouter"];
        assert_eq!(openrouter.api_key, "sk-live");
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
        let maestro_paths = MaestroPaths::new(dir.path());
        fs::create_dir_all(maestro_paths.maestro_dir()).unwrap();
        fs::write(maestro_paths.config_path(), "不是 JSON {{{").unwrap();
        let original = fs::read_to_string(maestro_paths.config_path()).unwrap();

        let mut store = Store::new(&maestro_paths);

        let err = store.get().unwrap_err();
        assert!(matches!(err, StoreError::Corrupt { .. }));
        assert!(String::from(err).contains(maestro_paths.config_path().to_str().unwrap()));

        assert!(
            store
                .create_provider(
                    "foo",
                    Provider {
                        base_url: Endpoints {
                            openai_completions: Option::Some("http://localhost:9".to_owned()),
                            anthropic_messages: Option::None,
                        },
                        api_key: String::new(),
                        models: Vec::new(),
                    }
                )
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(maestro_paths.config_path()).unwrap(),
            original
        );
    }

    #[test]
    fn unsupported_version_reports_error_with_path_and_refuses_writes() {
        let dir = tempfile::tempdir().unwrap();
        let maestro_paths = MaestroPaths::new(dir.path());
        fs::create_dir_all(maestro_paths.maestro_dir()).unwrap();
        fs::write(
            maestro_paths.config_path(),
            r#"{"version":99,"providers":{}}"#,
        )
        .unwrap();
        let original = fs::read_to_string(maestro_paths.config_path()).unwrap();

        let mut store = Store::new(&maestro_paths);

        let err = store.get().unwrap_err();
        assert!(matches!(err, StoreError::UnsupportedVersion { .. }));
        assert!(String::from(err).contains(maestro_paths.config_path().to_str().unwrap()));

        assert!(
            store
                .create_provider(
                    "foo",
                    Provider {
                        base_url: Endpoints {
                            openai_completions: Option::Some("http://localhost:9".to_owned()),
                            anthropic_messages: Option::None,
                        },
                        api_key: String::new(),
                        models: Vec::new(),
                    }
                )
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(maestro_paths.config_path()).unwrap(),
            original
        );
    }

    #[test]
    fn duplicate_slug_is_rejected_and_original_kept() {
        let dir = tempfile::tempdir().unwrap();
        let maestro_paths = MaestroPaths::new(dir.path());
        let mut store = Store::new(&maestro_paths);

        store
            .create_provider(
                "foo",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:9/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: String::new(),
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
                    api_key: String::new(),
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
        let maestro_paths = MaestroPaths::new(dir.path());
        let mut store = Store::new(&maestro_paths);

        store
            .create_provider(
                "foo",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::None,
                        anthropic_messages: Option::Some("http://127.0.0.1:8080".to_owned()),
                    },
                    api_key: String::new(),
                    models: Vec::new(),
                },
            )
            .unwrap();

        assert!(maestro_paths.config_path().exists());
    }

    #[test]
    fn provider_with_models_round_trips_through_disk_order_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let maestro_paths = MaestroPaths::new(dir.path());
        let mut store = Store::new(&maestro_paths);
        let models = vec![
            ModelEntry {
                id: "accounts/fireworks/models/llama3.1".to_owned(),
                display_name: Option::None,
            },
            ModelEntry {
                id: "Z-model".to_owned(),
                display_name: Option::Some("Z Model".to_owned()),
            },
            ModelEntry {
                id: "gpt-4o".to_owned(),
                display_name: Option::None,
            },
        ];

        store
            .create_provider(
                "openrouter",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("https://api.example.com/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: String::new(),
                    models: models.clone(),
                },
            )
            .unwrap();

        let reopened = Store::new(&maestro_paths);
        let provider = &reopened.get().unwrap().providers["openrouter"];
        assert_eq!(
            provider.models, models,
            "含模型的 Provider 保存/加载往返一致、保序，display_name 允许为 null"
        );
    }

    #[test]
    fn same_model_id_can_exist_in_different_providers() {
        let dir = tempfile::tempdir().unwrap();
        let maestro_paths = MaestroPaths::new(dir.path());
        let mut store = Store::new(&maestro_paths);
        let gpt_4o = || ModelEntry {
            id: "gpt-4o".to_owned(),
            display_name: Option::None,
        };

        store
            .create_provider(
                "openai",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("https://api.openai.com/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: String::new(),
                    models: vec![gpt_4o()],
                },
            )
            .unwrap();
        store
            .create_provider(
                "gateway",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://127.0.0.1:8080/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: String::new(),
                    models: vec![
                        gpt_4o(),
                        ModelEntry {
                            id: "gpt-4o-mini".to_owned(),
                            display_name: Option::None,
                        },
                    ],
                },
            )
            .unwrap();

        let reopened = Store::new(&maestro_paths);
        let config = reopened.get().unwrap();
        assert_eq!(config.providers["openai"].models, vec![gpt_4o()]);
        assert_eq!(
            config.providers["gateway"].models,
            vec![
                gpt_4o(),
                ModelEntry {
                    id: "gpt-4o-mini".to_owned(),
                    display_name: Option::None,
                }
            ]
        );
    }

    #[test]
    fn store_does_not_validate_model_ids() {
        // 业务校验（非空、Provider 内唯一）由前端内联完成；存储层原样保存，不拦不补。
        let dir = tempfile::tempdir().unwrap();
        let maestro_paths = MaestroPaths::new(dir.path());
        let mut store = Store::new(&maestro_paths);

        store
            .create_provider(
                "foo",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Option::Some("http://localhost:9/v1".to_owned()),
                        anthropic_messages: Option::None,
                    },
                    api_key: String::new(),
                    models: vec![
                        ModelEntry {
                            id: "gpt-4o".to_owned(),
                            display_name: Option::None,
                        },
                        ModelEntry {
                            id: "GPT-4O".to_owned(),
                            display_name: Option::None,
                        },
                        ModelEntry {
                            id: String::new(),
                            display_name: Option::None,
                        },
                    ],
                },
            )
            .unwrap();

        let reopened = Store::new(&maestro_paths);
        let models = &reopened.get().unwrap().providers["foo"].models;
        assert_eq!(models.len(), 3);
        assert_eq!(models[0].id, "gpt-4o");
        assert_eq!(models[1].id, "GPT-4O", "大小写敏感：大小写变体可并存");
        assert_eq!(models[2].id, "", "空 ID 同样不被存储层拦截");
    }
}
