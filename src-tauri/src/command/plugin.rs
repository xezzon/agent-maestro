use tauri::{AppHandle, Manager, State};

use crate::{
    AppStore, lock_store,
    plugin::{PluginApplyReport, PluginService, PluginView},
    store::StoreError,
};

/// 列出插件注册表（metadata 与加载状态）。
/// store 处于保护状态时报错，而非静默返回空列表误导用户。
#[tauri::command]
pub fn list_plugins(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
) -> Result<Vec<PluginView>, String> {
    let guard = lock_store(&store)?;
    guard.get().map_err(StoreError::message)?;
    Ok(service.list())
}

/// 启用/禁用插件。
#[tauri::command]
pub fn set_plugin_enabled(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
    source: String,
    enabled: bool,
) -> Result<(), String> {
    let mut guard = lock_store(&store)?;
    service.set_enabled(&mut guard, &source, enabled)
}

/// 添加插件（https 来源）：写条目后下载、校验、落位并装载。
///
/// 条目一旦写入即保留：安装失败进错误态，用「更新」重试。
#[tauri::command]
pub async fn add_plugin(app: AppHandle, url: String) -> Result<(), String> {
    on_install_pool(app, move |store, service| {
        let mut guard = store.lock()?;
        service.add_plugin(&mut guard, &url)
    })
    .await
}

/// 更新插件：无条件重新下载，成功才替换落位目录（失败时旧版本保持可用）。
#[tauri::command]
pub async fn update_plugin(app: AppHandle, source: String) -> Result<(), String> {
    on_install_pool(app, move |store, service| {
        let mut guard = store.lock()?;
        service.update_plugin(&mut guard, &source)
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
    let mut guard = lock_store(&store)?;
    service.remove_plugin(&mut guard, &source)
}

/// 重新加载：从磁盘重建插件注册表，不联网。
#[tauri::command]
pub fn reload_plugins(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
) -> Result<(), String> {
    let guard = lock_store(&store)?;
    service.reload(&guard);
    Ok(())
}

/// 应用到工具：调用所有已启用且加载成功的插件执行投影，
/// 返回逐插件结果（写入的文件、跳过的 Provider、失败原因）。
///
/// 投影前先释放 store 锁：插件执行时长不受应用控制，不得阻塞 Provider 命令。
#[tauri::command]
pub fn apply_providers(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
) -> Result<Vec<PluginApplyReport>, String> {
    let providers = {
        let guard = lock_store(&store)?;
        guard.get().map_err(StoreError::message)?.providers.clone()
    };
    Ok(service.apply(&providers))
}

/// 在阻塞线程池执行含网络下载的安装类操作：下载可能持续数秒，
/// 不得占用 IPC 线程。状态在阻塞任务内获取，避免跨线程持有引用。
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
