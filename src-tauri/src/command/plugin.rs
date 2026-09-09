use tauri::State;

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
