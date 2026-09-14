mod command;
mod paths;
mod plugin;
mod provider;
mod store;

use command::{
    add_plugin, apply_providers, create_provider, delete_provider, list_plugins, list_providers,
    reload_plugin, remove_plugin, set_plugin_enabled, update_provider,
};
use plugin::PluginService;
use std::sync::Mutex;
use store::{AppStore, Store};
use tauri::Manager;

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
        // 添加插件对话框用它选择本机 manifest.json（file 来源）。
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let home_path =
                dirs::home_dir().ok_or_else(|| "无法确定用户主目录（HOME）".to_owned())?;
            let maestro_paths = paths::MaestroPaths::new(&home_path);
            let store = Store::new(&maestro_paths);
            app.manage(AppStore {
                store: Mutex::new(store),
            });
            // 插件服务：`~` 无法确定时服务退化为不可用（命令层报错），
            // 与配置存储的保护状态语义一致。
            app.manage(PluginService::new(&maestro_paths));
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
            add_plugin,
            reload_plugin,
            remove_plugin,
            apply_providers
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
