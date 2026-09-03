mod provider;
mod store;

use std::sync::Mutex;

use store::Store;
use tauri::Manager;

/// 共享应用状态：配置存储（启动时加载进内存，变更后原子写回）。
struct AppStore(Mutex<Store>);

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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
