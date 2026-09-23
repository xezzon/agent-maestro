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

/// 撤销 AppImage 启动脚本对 X11 后端的强制指定（issue #46）。
///
/// AppImage 的 AppRun 会无条件 `export GDK_BACKEND=x11`（linuxdeploy-plugin-gtk
/// 注入的 apprun-hooks，其注释说明这是为了绕开 tauri#8541）。在 Wayland 会话下这会把
/// 应用按 XWayland 跑，X11 后端下 GTK3 又因会话里存在 `XMODIFIERS`（fcitx5 等输入法
/// 会设置）而默认选用 XIM 输入法模块——而 WebKitGTK 的 XIM 路径一旦有输入框获得焦点
/// 就停止出帧：窗口不再重绘、键盘无反应，仅缩放窗口还能刷新，表现为整个应用假死。
/// `tauri dev` 等启动方式没有这层覆盖（走 Wayland 后端与其输入法路径），故无此问题。
///
/// 只在 AppImage + Wayland 会话下动手：X11 会话本就该用 x11 后端，其它启动方式也不该
/// 覆盖用户自己的选择。
fn drop_appimage_x11_backend() {
    if std::env::var_os("APPIMAGE").is_none() || std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return;
    }
    if std::env::var("GDK_BACKEND").as_deref() != Ok("x11") {
        return;
    }
    // SAFETY: 在 run() 开头、GTK/WebKit 初始化之前调用，进程仍是单线程，没有其它
    // 线程并发读写环境变量。
    unsafe { std::env::remove_var("GDK_BACKEND") };
}

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
    // 必须早于 GTK/WebKit 初始化（后端在首次打开 GDK 显示时定下）。
    drop_appimage_x11_backend();
    install_panic_hook();
    if let Some(home) = dirs::home_dir() {
        paths::MaestroPaths::init(&home);
    }

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
    let builder = logging::attach(builder);

    builder
        .plugin(tauri_plugin_opener::init())
        // 添加插件对话框用它选择本机 manifest.json（file 来源）。
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let store = Store::new();
            app.manage(AppStore {
                store: RwLock::new(store),
            });
            app.manage(PluginService::new());

            // logger 已随插件 setup attach：补报降级原因与非法级别值。
            if let Some(reason) = logging::degraded_reason() {
                log::warn!("log file target unavailable, stdout only: {reason}");
            }
            if let Some(raw) = logging::invalid_level() {
                log::warn!("invalid MAESTRO_LOG_LEVEL value {raw:?}; using info");
            }

            // 启动时从配置条目重建注册表，并补装缺失的内置插件（落位 + 写条目，
            // 与其余来源同一安装管线；不联网，离线可用）。平台目录不可用时让
            // 启动失败，与主目录缺失的处理一致。
            let service = app.state::<PluginService>();
            let app_store = app.state::<AppStore>();
            service.startup(&app_store)?;
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
