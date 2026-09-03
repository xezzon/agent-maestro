use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use crate::provider::{Endpoints, Protocol, Provider};
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
    pub fn create_provider(
        &mut self,
        slug: &str,
        protocol: Protocol,
        base_url: &str,
    ) -> Result<(), StoreError> {
        let config = self.state.as_ref().map_err(Clone::clone)?;
        if config.providers.contains_key(slug) {
            return Err(StoreError::DuplicateSlug {
                slug: slug.to_owned(),
            });
        }
        let mut next = config.clone();
        next.providers.insert(
            slug.to_owned(),
            Provider {
                base_url: Endpoints::for_protocol(protocol, base_url),
                ..Provider::default()
            },
        );
        self.persist(&next)?;
        self.state = Ok(next);
        Ok(())
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

    #[test]
    fn unavailable_store_refuses_reads_and_writes() {
        let mut store = Store::unavailable("无法确定用户主目录（HOME）".to_owned());

        let err = store.get().unwrap_err();
        assert!(err.message().contains("无法确定用户主目录"));
        assert!(store
            .create_provider("foo", Protocol::OpenaiCompletions, "http://localhost:9")
            .is_err());
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
                        "openai-completions": "http://localhost:11434/v1",
                        "anthropic-messages": ""
                    },
                    "api_key": "",
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
                Protocol::OpenaiCompletions,
                "http://localhost:11434/v1",
            )
            .unwrap();

        let reopened = Store::open(path);
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
    fn persisted_file_omits_unconfigured_protocol_slots() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut store = Store::open(path.clone());

        store
            .create_provider(
                "ollama",
                Protocol::OpenaiCompletions,
                "http://localhost:11434/v1",
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
                .create_provider(slug, Protocol::OpenaiCompletions, "http://localhost:9/v1")
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
                Protocol::OpenaiCompletions,
                "http://localhost:11434/v1",
            )
            .unwrap();
        store
            .create_provider(
                "openrouter",
                Protocol::AnthropicMessages,
                "https://anthropic.example.com/v1",
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
                Protocol::OpenaiCompletions,
                "http://localhost:11434/v1",
            )
            .unwrap();

        let reopened = Store::open(path);
        let openrouter = &reopened.get().unwrap().providers["openrouter"];
        assert_eq!(
            openrouter.api_key,
            "secret://io.github.xezzon.agent-maestro/provider/openrouter/api_key"
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
            .create_provider("foo", Protocol::OpenaiCompletions, "http://localhost:9")
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
            .create_provider("foo", Protocol::OpenaiCompletions, "http://localhost:9")
            .is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn duplicate_slug_is_rejected_and_original_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut store = Store::open(path);

        store
            .create_provider("foo", Protocol::OpenaiCompletions, "http://localhost:9/v1")
            .unwrap();

        let err = store
            .create_provider("foo", Protocol::AnthropicMessages, "http://localhost:10")
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
            .create_provider("foo", Protocol::AnthropicMessages, "http://127.0.0.1:8080")
            .unwrap();

        assert!(path.exists());
    }
}
