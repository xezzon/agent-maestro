use super::{HostState, bindings};

/// logger import（issue #82 / ADR 0013）：每条消息按 level 写入应用日志，
/// debug 默认不落盘由全局级别旋钮（ADR 0008）承担。日志是诊断通道，
/// 只写不拦：绝不因记录失败让投影报错。
impl bindings::maestro::plugin::logger::Host for HostState {
    fn log(&mut self, level: bindings::maestro::plugin::logger::Level, message: String) {
        let log_level = map_plugin_level(level);
        // 前缀为宿主书写的英文标识；message 是插件的用户可见文案，保持原样。
        log::log!(log_level, "[plugin {} log] {message}", self.plugin_id);
    }
}

/// logger level → (应用日志 level, 标签) 的映射（ADR 0013）：error→ERROR、
/// warning→WARN、info→INFO、debug→DEBUG；debug 默认不落盘由全局级别旋钮
/// （ADR 0008）承担，这里只负责映射。
fn map_plugin_level(level: bindings::maestro::plugin::logger::Level) -> log::Level {
    use bindings::maestro::plugin::logger::Level as WitLevel;
    match level {
        WitLevel::Error => log::Level::Error,
        WitLevel::Warning => log::Level::Warn,
        WitLevel::Info => log::Level::Info,
        WitLevel::Debug => log::Level::Debug,
    }
}

#[cfg(test)]
mod tests {
    use super::map_plugin_level;

    /// logger level → (应用日志 level, 标签) 的映射（error→ERROR、warning→WARN、
    /// info→INFO、debug→DEBUG）；debug 默认不落盘由全局级别旋钮（ADR 0008）承担。
    #[test]
    fn map_plugin_level_maps_all_logger_levels() {
        use super::bindings::maestro::plugin::logger::Level as WitLevel;

        let level = map_plugin_level(WitLevel::Error);
        assert!(matches!(level, log::Level::Error));
        let level = map_plugin_level(WitLevel::Warning);
        assert!(matches!(level, log::Level::Warn));
        let level = map_plugin_level(WitLevel::Info);
        assert!(matches!(level, log::Level::Info));
        let level = map_plugin_level(WitLevel::Debug);
        assert!(matches!(level, log::Level::Debug));
    }
}
