use tauri::State;

use crate::{
    AppStore, lock_store,
    plugin::{PluginApplyReport, PluginService, PluginView},
    store::StoreError,
};

/// 列出插件注册表（metadata 与加载状态）。
#[tauri::command]
pub fn list_plugins(service: State<'_, PluginService>) -> Vec<PluginView> {
    service.list()
}

/// 添加 Git 来源插件：先落配置条目，随后下载/加载；
/// 失败保留条目、插件进错误态，可用「更新」重试。
#[tauri::command]
pub fn add_plugin(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
    source: String,
) -> Result<(), String> {
    validate_git_source(&source)?;
    let mut guard = lock_store(&store)?;
    service.add(&mut guard, &source)
}

/// 对 Git 来源插件执行「更新」：拉取最新版本，失败时旧版本继续可用。
#[tauri::command]
pub fn update_plugin(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
    source: String,
) -> Result<(), String> {
    let mut guard = lock_store(&store)?;
    service.update(&mut guard, &source)
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
#[tauri::command]
pub fn apply_providers(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
) -> Result<Vec<PluginApplyReport>, String> {
    let guard = lock_store(&store)?;
    let providers = &guard.get().map_err(StoreError::message)?.providers;
    Ok(service.apply(providers))
}

/// Git 来源 v1 仅匿名 HTTPS（本地路径仅供测试代码使用）。
fn validate_git_source(source: &str) -> Result<(), String> {
    let rest = source
        .strip_prefix("https://")
        .filter(|rest| !rest.is_empty())
        .ok_or_else(|| "Git 来源仅支持匿名 HTTPS 地址".to_owned())?;
    if rest.chars().any(char::is_whitespace) {
        return Err("Git 来源地址不合法".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_git_source;

    #[test]
    fn git_source_must_be_https() {
        assert!(validate_git_source("https://example.com/some-plugin.git").is_ok());
        assert!(validate_git_source("http://example.com/some-plugin.git").is_err());
        assert!(validate_git_source("git@github.com:user/repo.git").is_err());
        assert!(validate_git_source("file:///tmp/repo").is_err());
        assert!(validate_git_source("https://").is_err());
        assert!(validate_git_source("不是地址").is_err());
        assert!(validate_git_source("https://exa mple.com/repo.git").is_err());
    }
}
