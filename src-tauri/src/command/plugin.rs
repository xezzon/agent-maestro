use tauri::{AppHandle, Manager, State};

use crate::{
    AppStore,
    plugin::{PluginApplyReport, PluginService, PluginView},
};

/// 列出插件注册表（metadata 与加载状态）。
/// store 处于保护状态时报错，而非静默返回空列表误导用户；
/// 该检查由 `service.list` 内部的短锁完成，此处不得再持 store 锁——
/// 否则与 `list` 内部加锁构成同线程重入，死锁。
#[tauri::command]
pub fn list_plugins(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
) -> Result<Vec<PluginView>, String> {
    service.list(&store)
}

/// 启用/禁用插件。
#[tauri::command]
pub fn set_plugin_enabled(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
    source: String,
    enabled: bool,
) -> Result<(), String> {
    service.set_enabled(&store, &source, enabled)
}

/// 添加插件（来源为指向 manifest.json 的 https 地址或本机绝对路径）：获取 manifest 与
/// wasm、校验并落位，成功后才写入条目。
///
/// 失败不写条目、不落位，原因直接返回界面；修复后重新添加即可（见 ADR 0006）。
#[tauri::command]
pub async fn add_plugin(app: AppHandle, source: String) -> Result<(), String> {
    on_install_pool(app, move |store, service| {
        service.add_plugin(store, &source)
    })
    .await
}

/// 重新加载插件：按配置中的来源重新获取 manifest 与 wasm，成功才替换落位目录
/// （失败时旧版本保持可用）。
#[tauri::command]
pub async fn reload_plugin(app: AppHandle, source: String) -> Result<(), String> {
    on_install_pool(app, move |store, service| {
        service.reload_plugin(store, &source)
    })
    .await
}

/// 移除插件：删配置条目与落位目录（幂等），不联网。
#[tauri::command]
pub fn remove_plugin(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
    source: String,
) -> Result<(), String> {
    service.remove_plugin(&store, &source)
}

/// 应用到工具：调用所有已启用且加载成功的插件执行投影，
/// 返回逐插件结果（写入的文件、跳过的 Provider、失败原因）。
///
/// store 锁在读取 providers 后即释放，注册表锁也仅用于快照：
/// 插件执行时长不受应用控制，执行全程不持锁，不得阻塞其它命令。
#[tauri::command]
pub fn apply_providers(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
) -> Result<Vec<PluginApplyReport>, String> {
    let providers = {
        let guard = store.lock()?;
        guard.get()?.providers.clone()
    };
    service.write_providers(&providers)
}

/// 在阻塞线程池执行含网络下载的安装类操作：下载可能持续数秒，
/// 不得占用 IPC 线程。状态在阻塞任务内获取，避免跨线程持有引用。
///
/// 配置存储的锁由插件服务按短临界区自行获取：这里绝不代为持锁，
/// 下载、校验与落位全程不阻塞其它命令。
async fn on_install_pool<T, F>(app: AppHandle, task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&AppStore, &PluginService) -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let store = app.state::<AppStore>();
        let service = app.state::<PluginService>();
        task(&store, &service)
    })
    .await
    .map_err(|e| format!("插件安装任务执行失败：{e}"))?
}
