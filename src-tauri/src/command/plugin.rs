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

/// 添加 Git 来源插件：先落配置条目，随后下载/加载；
/// 失败保留条目、插件进错误态，可用「更新」重试。
///
/// 服务内部只短暂持有 store 锁，克隆与 WASM 校验期间不阻塞其他命令。
#[tauri::command]
pub fn add_plugin(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
    source: String,
) -> Result<(), String> {
    service.add(&store.store, &source)
}

/// 对 Git 来源插件执行「更新」：拉取最新版本，失败时旧版本继续可用。
#[tauri::command]
pub fn update_plugin(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
    source: String,
) -> Result<(), String> {
    service.update(&store.store, &source)
}

/// 移除 Git 插件：配置条目与插件目录一并清理。内置插件不可移除。
#[tauri::command]
pub fn remove_plugin(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
    source: String,
) -> Result<(), String> {
    let mut guard = lock_store(&store)?;
    service.remove(&mut guard, &source)
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
