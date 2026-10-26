mod plugin;
mod preferences;
mod provider;
mod theme;
mod variables;

pub(crate) use plugin::{
    add_plugin, apply_providers, get_plugin_config, list_plugins, reload_plugin, remove_plugin,
    set_plugin_config, set_plugin_enabled,
};
pub(crate) use preferences::{get_preferences, update_preferences};
pub(crate) use provider::{
    create_provider, delete_provider, list_providers, set_provider_enabled, update_provider,
};
pub(crate) use theme::host_requires_pinned_theme;
pub(crate) use variables::{list_variables, set_variables};

/// 命令层结果行（ADR 0008）：成功 INFO 携带识别参数（slug/source），失败 ERROR
/// 携带原因。命令 payload（如携带明文 api_key 的 `ProviderRequest`）一律不进
/// 日志，识别参数由调用方以 `key=value` 形式给出。
///
/// `context` 为 `None` 即无识别参数：成功行是「命令 ok」，失败行是「命令 failed:
/// 原因」，两者都不带尾随的冒号与空格。
fn log_outcome<T>(command: &str, context: Option<&str>, outcome: &Result<T, String>) {
    // 识别参数收成一个后缀：有时是「: 识别参数」，无识别参数即空串——成功行
    // 因此不留尾随的冒号与空格。
    let suffix = context.map(|c| format!(": {c}")).unwrap_or_default();
    match outcome {
        Ok(_) => log::info!("{command} ok{suffix}"),
        Err(reason) => log::error!("{command} failed{suffix}: {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::log_outcome;
    use crate::logging::capture::capture_window;

    /// 四个分支逐字钉住（ADR 0008 的共享日志契约此前零测试）：
    /// 成功/失败 × 有/无识别参数。无识别参数的行不带尾随冒号与空格。
    #[test]
    fn log_outcome_writes_the_four_branches() {
        let window = capture_window();

        log_outcome("list_providers", None, &Ok::<(), String>(()));
        log_outcome(
            "create_provider",
            Some("slug=ollama"),
            &Ok::<(), String>(()),
        );
        log_outcome(
            "list_providers",
            None,
            &Err::<(), String>("原因".to_owned()),
        );
        log_outcome(
            "create_provider",
            Some("slug=ollama"),
            &Err::<(), String>("原因".to_owned()),
        );

        let lines = window.lines();
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert!(lines[0].ends_with("list_providers ok"), "{lines:?}");
        assert!(
            lines[1].ends_with("create_provider ok: slug=ollama"),
            "{lines:?}"
        );
        assert!(
            lines[2].ends_with("list_providers failed: 原因"),
            "{lines:?}"
        );
        assert!(
            lines[3].ends_with("create_provider failed: slug=ollama: 原因"),
            "{lines:?}"
        );
    }

    /// 无识别参数的成功行不带尾随冒号与空格——`"apply_providers ok: "` 那种
    /// 尾随空格正是本条要防的回归。
    #[test]
    fn log_outcome_without_context_has_no_trailing_separator() {
        let window = capture_window();

        log_outcome("apply_providers", None, &Ok::<(), String>(()));

        let lines = window.lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        let message = lines[0].split_once("] ").unwrap().1;
        assert_eq!(message, "apply_providers ok", "{lines:?}");
    }
}
