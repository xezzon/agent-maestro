mod provider;
mod store;

use std::collections::BTreeMap;
use std::sync::Mutex;

use provider::Provider;
use store::{Store, StoreError};
use tauri::{Manager, State};

/// 共享应用状态：配置存储（启动时加载进内存，变更后原子写回）。
struct AppStore(Mutex<Store>);

fn lock<'a>(store: &'a State<'a, AppStore>) -> Result<std::sync::MutexGuard<'a, Store>, String> {
    store.0.lock().map_err(|_| "配置存储不可用".to_owned())
}

#[tauri::command]
fn list_providers(store: State<'_, AppStore>) -> Result<BTreeMap<String, Provider>, String> {
    let guard = lock(&store)?;
    let config = guard.get().map_err(StoreError::message)?;
    Ok(config.providers.clone())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let store = match store::default_path() {
                Ok(path) => Store::open(path),
                Err(detail) => Store::unavailable(detail),
            };
            app.manage(AppStore(Mutex::new(store)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![list_providers])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
