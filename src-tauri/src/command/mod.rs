mod plugin;
mod provider;

pub use plugin::{
    add_plugin, apply_providers, list_plugins, reload_plugin, remove_plugin, set_plugin_enabled,
};
pub use provider::{create_provider, delete_provider, list_providers, update_provider};

/// 命令层结果行（ADR 0008）：成功 INFO 携带识别参数（slug/source），失败 ERROR
/// 携带原因。命令 payload（如携带明文 api_key 的 `ProviderRequest`）一律不进
/// 日志，识别参数由调用方以 `key=value` 形式给出；`context` 为空即无识别参数。
pub(crate) fn log_outcome<T>(command: &str, context: &str, outcome: &Result<T, String>) {
    match outcome {
        Ok(_) if context.is_empty() => log::info!("{command} ok"),
        Ok(_) => log::info!("{command} ok: {context}"),
        Err(reason) if context.is_empty() => log::error!("{command} failed: {reason}"),
        Err(reason) => log::error!("{command} failed: {context}: {reason}"),
    }
}
