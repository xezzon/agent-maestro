//! Data model for the Maestro configuration (`~/.maestro/config.json`).
//!
//! The config file is the single source of truth for Provider and Plugin
//! entries. Secrets never appear here in plaintext: a Provider's `api_key`
//! holds a `secret://<name>` reference whose actual value lives in the
//! system keychain (see `keychain.rs`).

use serde::{Deserialize, Serialize};

/// Version of the on-disk schema. Bump together with a migration when the
/// shape of `Config` changes incompatibly.
pub const CONFIG_VERSION: u32 = 1;

/// Prefix of a secret reference stored in `Provider::api_key`.
pub const SECRET_SCHEME: &str = "secret";

/// The identifiers Maestro accepts for provider/plugin ids and model ids.
/// Kept deliberately narrow so a `secret://<id>` reference stays well-formed.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// Build a secret reference, e.g. `secret://openai`.
pub fn secret_ref(name: &str) -> String {
    format!("{SECRET_SCHEME}://{name}")
}

/// Extract the referenced name from a secret reference, or `None` if the
/// string is not a well-formed `secret://<name>` reference.
pub fn secret_ref_name(reference: &str) -> Option<&str> {
    reference
        .strip_prefix(&format!("{SECRET_SCHEME}://"))
        .filter(|name| !name.is_empty())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub version: u32,
    #[serde(default)]
    pub providers: Vec<Provider>,
    #[serde(default)]
    pub plugins: Vec<Plugin>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            providers: Vec::new(),
            plugins: Vec::new(),
        }
    }
}

impl Config {
    /// Validate the schema of the whole config. Called before persisting so
    /// Maestro never writes an invalid file, and on load so the UI can report
    /// a hand-edited file that no longer matches the schema.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != CONFIG_VERSION {
            return Err(format!(
                "unsupported config version {} (expected {CONFIG_VERSION})",
                self.version
            ));
        }
        let mut provider_ids: Vec<&str> = Vec::new();
        for provider in &self.providers {
            if !is_valid_id(&provider.id) {
                return Err(format!("provider id {:?} is invalid", provider.id));
            }
            if provider_ids.contains(&provider.id.as_str()) {
                return Err(format!("duplicate provider id {:?}", provider.id));
            }
            provider_ids.push(&provider.id);
            provider.validate()?;
        }
        let mut plugin_ids: Vec<&str> = Vec::new();
        for plugin in &self.plugins {
            if !is_valid_id(&plugin.id) {
                return Err(format!("plugin id {:?} is invalid", plugin.id));
            }
            if plugin_ids.contains(&plugin.id.as_str()) {
                return Err(format!("duplicate plugin id {:?}", plugin.id));
            }
            plugin_ids.push(&plugin.id);
            plugin.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Provider {
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub base_urls: BaseUrls,
    #[serde(default)]
    pub models: Vec<Model>,
    /// `secret://<name>` reference, never plaintext.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

impl Provider {
    fn validate(&self) -> Result<(), String> {
        self.base_urls.validate()?;
        let mut model_ids: Vec<&str> = Vec::new();
        for model in &self.models {
            if !is_valid_id(&model.id) {
                return Err(format!(
                    "provider {:?}: model id {:?} is invalid",
                    self.id, model.id
                ));
            }
            if model_ids.contains(&model.id.as_str()) {
                return Err(format!(
                    "provider {:?}: duplicate model id {:?}",
                    self.id, model.id
                ));
            }
            model_ids.push(&model.id);
        }
        if let Some(reference) = &self.api_key {
            match secret_ref_name(reference) {
                // The reference must point at this provider's own keychain
                // account (the id), not some other name.
                Some(name) if name == self.id && is_valid_id(name) => {}
                _ => {
                    return Err(format!(
                        "provider {:?}: api_key {:?} must be a secret://<id> reference matching the provider id",
                        self.id, reference
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Base URLs grouped by the protocol a Provider endpoint speaks.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BaseUrls {
    #[serde(
        default,
        rename = "openai-completions",
        skip_serializing_if = "Option::is_none"
    )]
    pub openai_completions: Option<String>,
    #[serde(
        default,
        rename = "anthropic-messages",
        skip_serializing_if = "Option::is_none"
    )]
    pub anthropic_messages: Option<String>,
}

impl BaseUrls {
    fn validate(&self) -> Result<(), String> {
        for (protocol, url) in [
            ("openai-completions", &self.openai_completions),
            ("anthropic-messages", &self.anthropic_messages),
        ] {
            if let Some(url) = url {
                if url.trim().is_empty() {
                    return Err(format!("{protocol} base URL must not be empty"));
                }
                if url::Url::parse(url).is_err() {
                    return Err(format!("{protocol} base URL {:?} is not a valid URL", url));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Model {
    pub id: String,
    /// Display name shown to the user; falls back to `id` when empty.
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Plugin {
    pub id: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Where the plugin comes from, e.g. `builtin` or a URL / path.
    pub source: String,
}

fn default_enabled() -> bool {
    true
}

impl Plugin {
    fn validate(&self) -> Result<(), String> {
        if self.source.trim().is_empty() {
            return Err(format!("plugin {:?}: source must not be empty", self.id));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> Config {
        Config {
            version: CONFIG_VERSION,
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
            plugins: vec![Plugin {
                id: "pi".into(),
                enabled: true,
                source: "builtin".into(),
            }],
        }
    }

    #[test]
    fn round_trips_through_json() {
        let config = sample_config();
        let json = serde_json::to_string_pretty(&config).unwrap();
        let decoded: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(config, decoded);
    }

    #[test]
    fn serializes_secret_as_reference_not_plaintext() {
        let config = sample_config();
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("secret://openai"));
        assert!(!json.contains("sk-plaintext"));
    }

    #[test]
    fn writes_protocol_keyed_base_urls() {
        let config = sample_config();
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"openai-completions\":\"https://api.openai.com/v1\""));
        assert!(!json.contains("openai_completions"));
    }

    #[test]
    fn omits_none_fields() {
        let no_key = Config {
            providers: vec![Provider {
                id: "a".into(),
                description: String::new(),
                base_urls: BaseUrls::default(),
                models: Vec::new(),
                api_key: None,
            }],
            ..Config::default()
        };
        let json = serde_json::to_string(&no_key).unwrap();
        assert!(!json.contains("api_key"));
        assert!(!json.contains("openai-completions"));
        assert!(!json.contains("anthropic-messages"));
    }

    #[test]
    fn rejects_unsupported_version() {
        let mut config = sample_config();
        config.version = 99;
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_invalid_provider_id() {
        let mut config = sample_config();
        config.providers[0].id = "has space".into();
        assert!(config.validate().is_err());
        config.providers[0].id = "".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_duplicate_ids() {
        let mut config = sample_config();
        let clone = config.providers[0].clone();
        config.providers.push(clone);
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_invalid_model_id() {
        let mut config = sample_config();
        config.providers[0].models[0].id = "".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_duplicate_model_ids() {
        let mut config = sample_config();
        config.providers[0].models.push(Model {
            id: "gpt-4o".into(),
            name: "GPT-4o again".into(),
        });
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_plaintext_api_key() {
        let mut config = sample_config();
        config.providers[0].api_key = Some("sk-plaintext".into());
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_api_key_referencing_another_name() {
        let mut config = sample_config();
        config.providers[0].api_key = Some(secret_ref("other-provider"));
        assert!(config.validate().is_err());

        // A reference whose name is not a valid id is also rejected.
        config.providers[0].api_key = Some(secret_ref("has space"));
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_invalid_base_url() {
        let mut config = sample_config();
        config.providers[0].base_urls.openai_completions = Some("not a url".into());
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_empty_plugin_source() {
        let mut config = sample_config();
        config.plugins[0].source = "  ".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn secret_ref_helpers() {
        assert_eq!(secret_ref("openai"), "secret://openai");
        assert_eq!(secret_ref_name("secret://openai"), Some("openai"));
        assert_eq!(secret_ref_name("secret://"), None);
        assert_eq!(secret_ref_name("sk-plaintext"), None);
    }

    #[test]
    fn is_valid_id_rules() {
        assert!(is_valid_id("openai"));
        assert!(is_valid_id("open-ai_v2.prod"));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("has space"));
        assert!(!is_valid_id("slashes/are/bad"));
        assert!(!is_valid_id("中文"));
    }
}
