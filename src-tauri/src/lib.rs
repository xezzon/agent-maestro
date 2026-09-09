mod command;
mod plugin;
mod provider;
mod store;

use command::{
    apply_providers, create_provider, delete_provider, list_plugins, list_providers,
    reload_plugins, set_plugin_enabled, update_provider,
};
use plugin::PluginService;
use std::sync::Mutex;
use store::Store;
use tauri::{Manager, State};

/// 共享应用状态：配置存储（启动时加载进内存，变更后原子写回）。
struct AppStore {
    store: Mutex<Store>,
}

fn lock_store<'a>(
    app: &'a State<'a, AppStore>,
) -> Result<std::sync::MutexGuard<'a, Store>, String> {
    app.store.lock().map_err(|_| "配置存储不可用".to_owned())
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
            app.manage(AppStore {
                store: Mutex::new(store),
            });
            // 插件服务：`~` 无法确定时服务退化为不可用（命令层报错），
            // 与配置存储的保护状态语义一致。
            app.manage(PluginService::default());
            // 启动时 upsert 内置插件条目并从磁盘重建注册表（不联网，离线可用）。
            let service = app.state::<PluginService>();
            let app_store = app.state::<AppStore>();
            service.startup(&app_store.store);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_providers,
            create_provider,
            update_provider,
            delete_provider,
            list_plugins,
            set_plugin_enabled,
            reload_plugins,
            apply_providers
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
