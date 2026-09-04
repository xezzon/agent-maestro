mod keychain;
mod provider;
mod store;

use std::collections::BTreeMap;
use std::sync::Mutex;

use provider::{Protocol, Provider};
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

#[tauri::command]
fn create_provider(
    store: State<'_, AppStore>,
    slug: String,
    protocol: Protocol,
    base_url: String,
) -> Result<(), String> {
    let mut guard = lock(&store)?;
    guard
        .create_provider(&slug, protocol, &base_url)
        .map_err(|e| e.message())
}

#[tauri::command]
fn update_provider(
    store: State<'_, AppStore>,
    slug: String,
    protocol: Protocol,
    base_url: String,
) -> Result<(), String> {
    let mut guard = lock(&store)?;
    guard
        .update_provider(&slug, protocol, &base_url)
        .map_err(|e| e.message())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    // 单实例运行：配置文件由唯一进程独占，避免多进程读-改-写相互覆盖 Provider。
    // 该插件必须先于其他插件注册。
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }));
    builder
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let store = match store::default_path() {
                Ok(path) => Store::open(path),
                Err(detail) => Store::unavailable(detail),
            };
            app.manage(AppStore(Mutex::new(store)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_providers,
            create_provider,
            update_provider
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
