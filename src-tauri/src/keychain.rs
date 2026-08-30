//! Secret storage backed by the system keychain.
//!
//! Per ADR-0002, the primary copy of a Secret lives in the system keychain
//! (macOS Keychain / Windows Credential Manager / Linux Secret Service) and
//! the config file only holds a `secret://<name>` reference. The service is
//! fixed to the app identifier; the account is the reference name.
//!
//! `SecretStore` is the seam that lets the rest of the crate talk to whatever
//! backend is installed; `InMemorySecretStore` is the test double used by
//! unit tests so no real keychain is touched.

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::Mutex;

/// The seam between service logic and the OS keychain.
pub trait SecretStore: Send + Sync {
    /// Store (or overwrite) the secret under `name`.
    fn set_secret(&self, name: &str, value: &str) -> Result<(), String>;
    /// Return the secret under `name`, or `None` when no such entry exists.
    fn get_secret(&self, name: &str) -> Result<Option<String>, String>;
    /// Delete the entry under `name`; missing entries are a no-op.
    fn delete_secret(&self, name: &str) -> Result<(), String>;
}

/// System keychain backend via the `keyring` crate. `service` is the app
/// identifier; each secret's account is the reference name.
pub struct KeyringSecretStore {
    service: String,
}

impl KeyringSecretStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }
}

impl SecretStore for KeyringSecretStore {
    fn set_secret(&self, name: &str, value: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(&self.service, name).map_err(|e| e.to_string())?;
        entry.set_password(value).map_err(|e| e.to_string())
    }

    fn get_secret(&self, name: &str) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new(&self.service, name).map_err(|e| e.to_string())?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn delete_secret(&self, name: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(&self.service, name).map_err(|e| e.to_string())?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// In-memory test double. Not for production use.
#[cfg(test)]
#[derive(Default)]
pub struct InMemorySecretStore {
    secrets: Mutex<HashMap<String, String>>,
}

#[cfg(test)]
impl SecretStore for InMemorySecretStore {
    fn set_secret(&self, name: &str, value: &str) -> Result<(), String> {
        self.secrets
            .lock()
            .map_err(|_| "in-memory secret store poisoned".to_string())?
            .insert(name.to_string(), value.to_string());
        Ok(())
    }

    fn get_secret(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self
            .secrets
            .lock()
            .map_err(|_| "in-memory secret store poisoned".to_string())?
            .get(name)
            .cloned())
    }

    fn delete_secret(&self, name: &str) -> Result<(), String> {
        self.secrets
            .lock()
            .map_err(|_| "in-memory secret store poisoned".to_string())?
            .remove(name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_store_round_trips() {
        let store = InMemorySecretStore::default();
        assert_eq!(store.get_secret("openai").unwrap(), None);

        store.set_secret("openai", "sk-test").unwrap();
        assert_eq!(store.get_secret("openai").unwrap(), Some("sk-test".into()));

        store.set_secret("openai", "sk-rotated").unwrap();
        assert_eq!(
            store.get_secret("openai").unwrap(),
            Some("sk-rotated".into())
        );

        store.delete_secret("openai").unwrap();
        assert_eq!(store.get_secret("openai").unwrap(), None);
    }

    #[test]
    fn deleting_missing_entry_is_a_no_op() {
        let store = InMemorySecretStore::default();
        store.delete_secret("never-existed").unwrap();
    }

    #[test]
    fn entries_are_isolated_by_name() {
        let store = InMemorySecretStore::default();
        store.set_secret("openai", "sk-1").unwrap();
        store.set_secret("anthropic", "sk-2").unwrap();
        assert_eq!(store.get_secret("openai").unwrap(), Some("sk-1".into()));
        assert_eq!(store.get_secret("anthropic").unwrap(), Some("sk-2".into()));
    }
}
