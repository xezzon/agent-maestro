mod command;
mod logging;
mod paths;
mod plugin;
mod provider;
mod store;

use command::{
    add_plugin, apply_providers, create_provider, delete_provider, list_plugins, list_providers,
    reload_plugin, remove_plugin, set_plugin_enabled, update_provider,
};
use plugin::PluginService;
use std::sync::RwLock;
use store::{AppStore, Store};
use tauri::Manager;

/// 未捕获 panic 记入日志（含 file:line 与消息），并以 `take_hook()` 链式调用
/// 原 hook 保住 stderr 默认行为。logger attach 之前的 panic 由原 hook 打到
/// stderr（`log::error!` 此时尚未生效）。命令层 panic 若无此 hook：IPC 调用
/// 永不返回、前端 promise 挂住，而日志里不留任何痕迹。
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        default_hook(info);
    }));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_panic_hook();
    let maestro_paths = dirs::home_dir().map(|home| paths::MaestroPaths::new(&home));

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
    // 日志设施（ADR 0008）：注册前对日志目录写探测，文件目标不可用时只装
    // Stdout，写不出日志绝不阻断启动。
    let builder = logging::attach(builder, maestro_paths.as_ref());

    builder
        .plugin(tauri_plugin_opener::init())
        // 添加插件对话框用它选择本机 manifest.json（file 来源）。
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let maestro_paths =
                maestro_paths.ok_or_else(|| "无法确定用户主目录（HOME）".to_owned())?;
            let store = Store::new(&maestro_paths);
            app.manage(AppStore {
                store: RwLock::new(store),
            });
            // 插件服务：`~` 无法确定时服务退化为不可用（命令层报错），
            // 与配置存储的保护状态语义一致。
            app.manage(PluginService::new(&maestro_paths));

            // logger 已随插件 setup attach：补报降级原因与非法级别值。
            if let Some(reason) = logging::degraded_reason() {
                log::warn!("log file target unavailable, stdout only: {reason}");
            }
            if let Some(raw) = logging::invalid_level() {
                log::warn!("invalid MAESTRO_LOG_LEVEL value {raw:?}; using info");
            }

            // 启动时从配置条目重建注册表，并补装缺失的内置插件（落位 + 写条目，
            // 与其余来源同一安装管线；不联网，离线可用）。
            let service = app.state::<PluginService>();
            let app_store = app.state::<AppStore>();
            service.startup(&app_store);
            match app_store.read() {
                Ok(guard) => match guard.get() {
                    Ok(config) => log::info!(
                        "startup complete: {} providers, {} plugins",
                        config.providers.len(),
                        config.plugins.len()
                    ),
                    Err(err) => {
                        log::error!("startup: config store unavailable: {}", String::from(err))
                    }
                },
                Err(err) => log::error!("startup: {err}"),
            }
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
