//! Business logic for mutating the config, plus the stateful `AppService`
//! facade used by the Tauri commands.

use serde::{Deserialize, Serialize};

use crate::config::{is_valid_id, secret_ref, BaseUrls, Config, Model, Plugin, Provider};
use crate::config_store::ConfigStore;
use crate::keychain::SecretStore;

/// Wire shape for creating or updating a Provider, as sent from the frontend.
///
/// `api_key` semantics:
/// - `None` — leave the stored key untouched (edit without entering one).
/// - `Some("")` — clear the stored key (delete from keychain, drop the ref).
/// - `Some(key)` — store `key` in the keychain and reference it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInput {
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub base_urls: BaseUrls,
    #[serde(default)]
    pub models: Vec<Model>,
    #[serde(default)]
    pub api_key: Option<String>,
}

/// Wire shape for creating or updating a Plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInput {
    pub id: String,
    #[serde(default)]
    pub enabled: bool,
    pub source: String,
}

/// Stateful facade over `ConfigStore` + `SecretStore`; owned by Tauri state.
pub struct AppService {
    store: ConfigStore,
    secrets: Box<dyn SecretStore>,
}

impl AppService {
    pub fn new(store: ConfigStore, secrets: Box<dyn SecretStore>) -> Self {
        Self { store, secrets }
    }

    pub fn get_config(&self) -> Result<Config, String> {
        self.store.load()
    }

    pub fn save_provider(&self, input: ProviderInput) -> Result<Config, String> {
        let id = input.id.trim().to_string();
        let mut config = self.store.load()?;
        // Only when the input changes the key is the keychain written; in that
        // case snapshot the previous entry so a failed config write can restore
        // it instead of leaving an orphaned (or lost) secret behind.
        let mut previous: Option<Option<String>> = None;
        if input.api_key.is_some() {
            previous = Some(self.secrets.get_secret(&id)?);
        }
        save_provider(&mut config, self.secrets.as_ref(), input)?;
        if let Err(e) = self.store.save(&config) {
            if let Some(previous) = previous {
                match previous {
                    Some(old) => {
                        let _ = self.secrets.set_secret(&id, &old);
                    }
                    None => {
                        let _ = self.secrets.delete_secret(&id);
                    }
                }
            }
            return Err(e);
        }
        Ok(config)
    }

    pub fn delete_provider(&self, id: &str) -> Result<Config, String> {
        let mut config = self.store.load()?;
        delete_provider(&mut config, self.secrets.as_ref(), id)?;
        self.store.save(&config)?;
        Ok(config)
    }

    pub fn save_plugin(&self, input: PluginInput) -> Result<Config, String> {
        let mut config = self.store.load()?;
        save_plugin(&mut config, input)?;
        self.store.save(&config)?;
        Ok(config)
    }

    pub fn delete_plugin(&self, id: &str) -> Result<Config, String> {
        let mut config = self.store.load()?;
        delete_plugin(&mut config, id)?;
        self.store.save(&config)?;
        Ok(config)
    }
}

/// Create or update a Provider. The id is the identity: updating an existing
/// id edits in place, any other id appends.
pub fn save_provider(
    config: &mut Config,
    secrets: &dyn SecretStore,
    input: ProviderInput,
) -> Result<(), String> {
    let id = input.id.trim().to_string();
    if !is_valid_id(&id) {
        return Err(format!("invalid provider id: {:?}", input.id));
    }
    for model in &input.models {
        if !is_valid_id(&model.id) {
            return Err(format!("invalid model id: {:?}", model.id));
        }
    }

    let existing = config.providers.iter().position(|p| p.id == id);

    // Resolve what happens to the key before mutating the config.
    let api_key = match &input.api_key {
        None => existing.and_then(|i| config.providers[i].api_key.clone()),
        Some(key) if key.trim().is_empty() => {
            // Explicit clear: best-effort keychain cleanup.
            let _ = secrets.delete_secret(&id);
            None
        }
        Some(key) => {
            secrets.set_secret(&id, key.trim())?;
            Some(secret_ref(&id))
        }
    };

    let provider = Provider {
        id,
        description: input.description.trim().to_string(),
        base_urls: BaseUrls {
            openai_completions: trim_opt(input.base_urls.openai_completions),
            anthropic_messages: trim_opt(input.base_urls.anthropic_messages),
        },
        models: input.models,
        api_key,
    };

    match existing {
        Some(i) => config.providers[i] = provider,
        None => config.providers.push(provider),
    }
    Ok(())
}

/// Remove a Provider and best-effort delete its keychain entry.
pub fn delete_provider(
    config: &mut Config,
    secrets: &dyn SecretStore,
    id: &str,
) -> Result<(), String> {
    let Some(i) = config.providers.iter().position(|p| p.id == id) else {
        return Err(format!("provider not found: {id:?}"));
    };
    config.providers.remove(i);
    // Best-effort keychain cleanup: the config deletion is the user's intent,
    // so a keychain hiccup must not block it (worst case a stale entry stays).
    let _ = secrets.delete_secret(id);
    Ok(())
}

/// Create or update a Plugin. Toggling enabled is just a save with the
/// flipped flag.
pub fn save_plugin(config: &mut Config, input: PluginInput) -> Result<(), String> {
    let id = input.id.trim().to_string();
    if !is_valid_id(&id) {
        return Err(format!("invalid plugin id: {:?}", input.id));
    }
    let source = input.source.trim().to_string();
    if source.is_empty() {
        return Err("plugin source must not be empty".to_string());
    }
    let plugin = Plugin {
        id,
        enabled: input.enabled,
        source,
    };
    match config.plugins.iter().position(|p| p.id == plugin.id) {
        Some(i) => config.plugins[i] = plugin,
        None => config.plugins.push(plugin),
    }
    Ok(())
}

/// Remove a Plugin.
pub fn delete_plugin(config: &mut Config, id: &str) -> Result<(), String> {
    let Some(i) = config.plugins.iter().position(|p| p.id == id) else {
        return Err(format!("plugin not found: {id:?}"));
    };
    config.plugins.remove(i);
    Ok(())
}

fn trim_opt(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{secret_ref_name, CONFIG_VERSION};
    use crate::keychain::InMemorySecretStore;
    use std::fs;
    use std::sync::Arc;

    // Arc keeps a handle on the store outside the service so tests can assert
    // on its contents after a (simulated) failed operation.
    impl SecretStore for Arc<InMemorySecretStore> {
        fn set_secret(&self, name: &str, value: &str) -> Result<(), String> {
            (**self).set_secret(name, value)
        }

        fn get_secret(&self, name: &str) -> Result<Option<String>, String> {
            (**self).get_secret(name)
        }

        fn delete_secret(&self, name: &str) -> Result<(), String> {
            (**self).delete_secret(name)
        }
    }

    fn store() -> InMemorySecretStore {
        InMemorySecretStore::default()
    }

    fn provider_input(id: &str, api_key: Option<&str>) -> ProviderInput {
        ProviderInput {
            id: id.into(),
            description: "desc".into(),
            base_urls: BaseUrls {
                openai_completions: Some("https://api.example.com/v1".into()),
                anthropic_messages: None,
            },
            models: vec![Model {
                id: "m1".into(),
                name: "Model 1".into(),
            }],
            api_key: api_key.map(String::from),
        }
    }

    fn plugin_input(id: &str, enabled: bool) -> PluginInput {
        PluginInput {
            id: id.into(),
            enabled,
            source: "builtin".into(),
        }
    }

    #[test]
    fn add_provider_with_key_stores_secret_and_references_it() {
        let mut config = Config::default();
        let secrets = store();
        save_provider(
            &mut config,
            &secrets,
            provider_input("openai", Some("sk-123")),
        )
        .unwrap();

        assert_eq!(config.providers.len(), 1);
        let provider = &config.providers[0];
        assert_eq!(provider.api_key.as_deref(), Some("secret://openai"));
        // Plaintext never lands in the config.
        assert!(secret_ref_name(provider.api_key.as_deref().unwrap()).is_some());
        assert_eq!(secrets.get_secret("openai").unwrap(), Some("sk-123".into()));
    }

    #[test]
    fn add_provider_without_key_has_no_ref() {
        let mut config = Config::default();
        let secrets = store();
        save_provider(&mut config, &secrets, provider_input("openai", None)).unwrap();
        assert_eq!(config.providers[0].api_key, None);
    }

    #[test]
    fn edit_without_api_key_keeps_existing_secret() {
        let mut config = Config::default();
        let secrets = store();
        save_provider(
            &mut config,
            &secrets,
            provider_input("openai", Some("sk-123")),
        )
        .unwrap();
        save_provider(&mut config, &secrets, provider_input("openai", None)).unwrap();

        assert_eq!(
            config.providers[0].api_key.as_deref(),
            Some("secret://openai")
        );
        assert_eq!(secrets.get_secret("openai").unwrap(), Some("sk-123".into()));
    }

    #[test]
    fn edit_with_new_key_rotates_secret() {
        let mut config = Config::default();
        let secrets = store();
        save_provider(
            &mut config,
            &secrets,
            provider_input("openai", Some("sk-old")),
        )
        .unwrap();
        save_provider(
            &mut config,
            &secrets,
            provider_input("openai", Some("sk-new")),
        )
        .unwrap();

        assert_eq!(
            config.providers[0].api_key.as_deref(),
            Some("secret://openai")
        );
        assert_eq!(secrets.get_secret("openai").unwrap(), Some("sk-new".into()));
    }

    #[test]
    fn edit_with_empty_key_clears_secret() {
        let mut config = Config::default();
        let secrets = store();
        save_provider(
            &mut config,
            &secrets,
            provider_input("openai", Some("sk-123")),
        )
        .unwrap();
        save_provider(&mut config, &secrets, provider_input("openai", Some(""))).unwrap();

        assert_eq!(config.providers[0].api_key, None);
        assert_eq!(secrets.get_secret("openai").unwrap(), None);
    }

    #[test]
    fn edit_keeps_position_and_updates_fields() {
        let mut config = Config::default();
        let secrets = store();
        save_provider(&mut config, &secrets, provider_input("a", None)).unwrap();
        save_provider(&mut config, &secrets, provider_input("b", None)).unwrap();
        save_provider(&mut config, &secrets, provider_input("a", None)).unwrap();

        assert_eq!(config.providers.len(), 2);
        assert_eq!(config.providers[0].id, "a");
        assert_eq!(config.providers[1].id, "b");
    }

    #[test]
    fn reject_invalid_provider_id() {
        let mut config = Config::default();
        let secrets = store();
        let err = save_provider(&mut config, &secrets, provider_input("bad id", None)).unwrap_err();
        assert!(err.contains("invalid provider id"));
    }

    #[test]
    fn reject_invalid_model_id() {
        let mut config = Config::default();
        let secrets = store();
        let mut input = provider_input("openai", None);
        input.models[0].id = "".into();
        let err = save_provider(&mut config, &secrets, input).unwrap_err();
        assert!(err.contains("invalid model id"));
    }

    #[test]
    fn delete_provider_removes_config_and_secret() {
        let mut config = Config::default();
        let secrets = store();
        save_provider(
            &mut config,
            &secrets,
            provider_input("openai", Some("sk-123")),
        )
        .unwrap();
        delete_provider(&mut config, &secrets, "openai").unwrap();

        assert!(config.providers.is_empty());
        assert_eq!(secrets.get_secret("openai").unwrap(), None);
    }

    #[test]
    fn delete_missing_provider_is_an_error() {
        let mut config = Config::default();
        let secrets = store();
        assert!(delete_provider(&mut config, &secrets, "nope").is_err());
    }

    #[test]
    fn add_plugin_and_toggle_enabled() {
        let mut config = Config::default();
        save_plugin(&mut config, plugin_input("pi", true)).unwrap();
        assert_eq!(config.plugins.len(), 1);
        assert!(config.plugins[0].enabled);

        save_plugin(&mut config, plugin_input("pi", false)).unwrap();
        assert_eq!(config.plugins.len(), 1);
        assert!(!config.plugins[0].enabled);
    }

    #[test]
    fn delete_plugin_removes_entry() {
        let mut config = Config::default();
        save_plugin(&mut config, plugin_input("pi", true)).unwrap();
        delete_plugin(&mut config, "pi").unwrap();
        assert!(config.plugins.is_empty());
        assert!(delete_plugin(&mut config, "pi").is_err());
    }

    #[test]
    fn reject_empty_plugin_source() {
        let mut config = Config::default();
        let mut input = plugin_input("pi", true);
        input.source = " ".into();
        assert!(save_plugin(&mut config, input).is_err());
    }

    #[test]
    fn app_service_persists_through_store() {
        let dir = tempfile::tempdir().unwrap();
        let service = AppService::new(
            ConfigStore::new(dir.path().join("config.json")),
            Box::new(store()),
        );

        let config = service
            .save_provider(provider_input("openai", Some("sk-123")))
            .unwrap();
        assert_eq!(
            config.providers[0].api_key.as_deref(),
            Some("secret://openai")
        );
        assert_eq!(config.version, CONFIG_VERSION);

        // A second service reading the same file sees the persisted state.
        let service2 = AppService::new(
            ConfigStore::new(dir.path().join("config.json")),
            Box::new(store()),
        );
        let loaded = service2.get_config().unwrap();
        assert_eq!(loaded, config);
    }

    #[test]
    fn app_service_restores_keychain_when_config_save_fails() {
        let dir = tempfile::tempdir().unwrap();
        // Making the target path a directory forces the atomic rename to fail.
        let path = dir.path().join("config.json");
        fs::create_dir(&path).unwrap();
        let secrets = Arc::new(InMemorySecretStore::default());
        let service = AppService::new(ConfigStore::new(path), Box::new(secrets.clone()));

        // New key written, config save fails: the orphaned entry is rolled back.
        assert!(service
            .save_provider(provider_input("openai", Some("sk-new")))
            .is_err());
        assert_eq!(secrets.get_secret("openai").unwrap(), None);

        // Rotation fails: the previous value is restored, not lost.
        secrets.set_secret("openai", "sk-old").unwrap();
        assert!(service
            .save_provider(provider_input("openai", Some("sk-new")))
            .is_err());
        assert_eq!(secrets.get_secret("openai").unwrap(), Some("sk-old".into()));
    }
}
