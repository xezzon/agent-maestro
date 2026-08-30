// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/

mod config;
mod config_store;
mod keychain;
mod service;

use std::sync::Mutex;

use config::Config;
use config_store::ConfigStore;
use keychain::{KeyringSecretStore, SecretStore};
use service::{AppService, PluginInput, ProviderInput};
use tauri::Manager;
use tauri::State;

/// Tauri-managed application state. `lock` serializes config mutations so
/// concurrent commands cannot interleave load-modify-save cycles.
struct AppState {
    service: AppService,
    lock: Mutex<()>,
}

impl AppState {
    fn guard(&self) -> Result<std::sync::MutexGuard<'_, ()>, String> {
        self.lock
            .lock()
            .map_err(|_| "internal state lock poisoned".to_string())
    }
}

#[tauri::command]
async fn get_config(state: State<'_, AppState>) -> Result<Config, String> {
    let _guard = state.guard()?;
    state.service.get_config()
}

#[tauri::command]
async fn save_provider(
    state: State<'_, AppState>,
    provider: ProviderInput,
) -> Result<Config, String> {
    let _guard = state.guard()?;
    state.service.save_provider(provider)
}

#[tauri::command]
async fn delete_provider(state: State<'_, AppState>, id: String) -> Result<Config, String> {
    let _guard = state.guard()?;
    state.service.delete_provider(&id)
}

#[tauri::command]
async fn save_plugin(state: State<'_, AppState>, plugin: PluginInput) -> Result<Config, String> {
    let _guard = state.guard()?;
    state.service.save_plugin(plugin)
}

#[tauri::command]
async fn delete_plugin(state: State<'_, AppState>, id: String) -> Result<Config, String> {
    let _guard = state.guard()?;
    state.service.delete_plugin(&id)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let path = ConfigStore::default_path()?;
            let secrets: Box<dyn SecretStore> =
                Box::new(KeyringSecretStore::new(app.config().identifier.clone()));
            app.manage(AppState {
                service: AppService::new(ConfigStore::new(path), secrets),
                lock: Mutex::new(()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_provider,
            delete_provider,
            save_plugin,
            delete_plugin
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
