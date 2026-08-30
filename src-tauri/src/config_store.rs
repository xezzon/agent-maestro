//! Persistence of the Maestro config file at `~/.maestro/config.json`.

use std::fs;
use std::path::PathBuf;

use crate::config::Config;

/// Loads and saves the config file. Paths are injectable so tests can point
/// at a temp directory instead of the real home.
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The canonical location: `~/.maestro/config.json`.
    pub fn default_path() -> Result<PathBuf, String> {
        let home = dirs::home_dir().ok_or("cannot determine the home directory")?;
        Ok(home.join(".maestro").join("config.json"))
    }

    /// Load the config. A missing file yields an empty default config; a
    /// present but unreadable/malformed file is an error surfaced to the UI.
    pub fn load(&self) -> Result<Config, String> {
        if !self.path.exists() {
            return Ok(Config::default());
        }
        let raw = fs::read_to_string(&self.path)
            .map_err(|e| format!("failed to read {}: {e}", self.path.display()))?;
        let config: Config =
            serde_json::from_str(&raw).map_err(|e| format!("invalid config JSON: {e}"))?;
        config
            .validate()
            .map_err(|e| format!("config {} is invalid: {e}", self.path.display()))?;
        Ok(config)
    }

    /// Validate, then persist atomically (write to a sibling temp file and
    /// rename over the target) so a crash mid-write cannot corrupt the file.
    pub fn save(&self, config: &Config) -> Result<(), String> {
        config.validate()?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(config)
            .map_err(|e| format!("failed to serialize config: {e}"))?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(|e| format!("failed to write {}: {e}", tmp.display()))?;
        fs::rename(&tmp, &self.path)
            .map_err(|e| format!("failed to move {} into place: {e}", tmp.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{secret_ref, BaseUrls, Model, Provider};

    fn sample_config() -> Config {
        Config {
            version: 1,
            providers: vec![Provider {
                id: "openai".into(),
                description: "OpenAI".into(),
                base_urls: BaseUrls {
                    openai_completions: Some("https://api.openai.com/v1".into()),
                    anthropic_messages: None,
                },
                models: vec![Model {
                    id: "gpt-4o".into(),
                    name: "GPT-4o".into(),
                }],
                api_key: Some(secret_ref("openai")),
            }],
            plugins: vec![crate::config::Plugin {
                id: "pi".into(),
                enabled: true,
                source: "builtin".into(),
            }],
        }
    }

    #[test]
    fn load_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("config.json"));
        assert_eq!(store.load().unwrap(), Config::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let store = ConfigStore::new(path.clone());
        let config = sample_config();
        store.save(&config).unwrap();
        assert_eq!(store.load().unwrap(), config);
        assert!(path.exists());
    }

    #[test]
    fn save_creates_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b").join("config.json");
        let store = ConfigStore::new(nested.clone());
        store.save(&sample_config()).unwrap();
        assert!(nested.exists());
    }

    #[test]
    fn load_rejects_invalid_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, r#"{"version": 1, "providers": [{"id": ""}]}"#).unwrap();
        let store = ConfigStore::new(path);
        let err = store.load().unwrap_err();
        assert!(err.contains("invalid"));
    }

    #[test]
    fn load_rejects_malformed_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, "{ not json").unwrap();
        let store = ConfigStore::new(path);
        assert!(store.load().is_err());
    }

    #[test]
    fn save_refuses_invalid_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let store = ConfigStore::new(path.clone());
        let mut config = sample_config();
        config.providers[0].api_key = Some("sk-plaintext".into());
        assert!(store.save(&config).is_err());
        assert!(!path.exists());
    }
}
