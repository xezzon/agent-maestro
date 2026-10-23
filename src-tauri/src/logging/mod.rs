//! 日志设施（ADR 0008）：`tauri-plugin-log`（`log` crate 门面）作为唯一日志门面，
//! 落盘 `~/.maestro/logs/agent-maestro.log`，开发期同时输出 stdout。
//!
//! 写不出日志绝不阻断启动：注册日志插件之前对日志目录做写探测，失败则只装
//! Stdout target，应用照常启动——降级原因由应用 setup 阶段（logger 就绪后）
//! 经 [`degraded_reason`] 补一条 WARN。

use std::{fs, sync::OnceLock};

use log::LevelFilter;
use tauri_plugin_log::{
    Builder as LogBuilder, FileOpenStrategy, RotationStrategy, Target, TargetKind,
};

use crate::paths::Paths;

#[cfg(test)]
pub(crate) mod capture;

/// 日志文件名（固定，落位 `~/.maestro/logs/` 下）。
const LOG_FILE_NAME: &str = "agent-maestro.log";
/// 单文件上限：超出即轮转（默认 40 KB 不可用，插件的刷屏路径能写爆磁盘）。
const MAX_FILE_SIZE: u128 = 1024 * 1024;
/// 轮转保留的归档份数：15 份归档 + 1 份活动 ≈ 16 MiB 封顶。
/// `KeepOne` 会在轮转时直接删除超限文件，恰在最需要证据时销毁它。
const KEEP_ARCHIVES: usize = 15;
/// 级别默认 INFO（开发与发布一致）；`MAESTRO_LOG_LEVEL` 可覆盖。
const DEFAULT_LEVEL: LevelFilter = LevelFilter::Info;

/// 日志文件目标不可用的原因（probe 失败或主目录缺失）；logger 就绪后回报。
static DEGRADED: OnceLock<String> = OnceLock::new();
/// `MAESTRO_LOG_LEVEL` 的非法值；logger 就绪后回报并忽略。
static INVALID_LEVEL: OnceLock<String> = OnceLock::new();

/// 文件目标不可用的原因；由应用 setup 阶段（logger 已就绪）读取并打 WARN。
pub(crate) fn degraded_reason() -> Option<&'static str> {
    DEGRADED.get().map(String::as_str)
}

/// 环境变量里的非法级别值；由应用 setup 阶段读取并打 WARN。
pub(crate) fn invalid_level() -> Option<&'static str> {
    INVALID_LEVEL.get().map(String::as_str)
}

/// 解析裸级别名 `error|warn|info|debug|trace`；非法值返回 `None`。
/// 不复用 `RUST_LOG`：没有 `EnvFilter`，支持 `module=level` 指令只会静默误解析。
fn parse_level(raw: &str) -> Option<LevelFilter> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "error" => Some(LevelFilter::Error),
        "warn" => Some(LevelFilter::Warn),
        "info" => Some(LevelFilter::Info),
        "debug" => Some(LevelFilter::Debug),
        "trace" => Some(LevelFilter::Trace),
        _ => None,
    }
}

/// 写探测：目录可创建、日志文件可打开才算文件目标可用。
fn probe_log_dir(paths: &Paths) -> Result<(), String> {
    let dir = paths.logs_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let file = dir.join(LOG_FILE_NAME);
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
        .map_err(|e| format!("open {}: {e}", file.display()))?;
    Ok(())
}

/// 注册日志插件：Stdout + 文件 target，Append 跨启动连续写，UTC + 插件默认行格式
/// （`.timezone_strategy()` 与 `.format()` 都不调，前者会重写 format 颠倒字段顺序）。
/// 级别是单一全局旋钮：默认 INFO，`MAESTRO_LOG_LEVEL` 接受裸级别名覆盖，
/// 非法值忽略并在 logger 就绪后补 WARN（不写入 `config.json`）。
///
/// 必须在单实例插件之后调用（该插件须先于其他插件注册）。
pub(crate) fn attach(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    let (level, invalid) = match std::env::var("MAESTRO_LOG_LEVEL") {
        Ok(raw) => match parse_level(&raw) {
            Some(level) => (level, None),
            None => (DEFAULT_LEVEL, Some(raw)),
        },
        Err(_) => (DEFAULT_LEVEL, None),
    };
    if let Some(raw) = invalid {
        let _ = INVALID_LEVEL.set(raw);
    }

    let mut targets = vec![Target::new(TargetKind::Stdout)];
    match Paths::try_get() {
        Some(paths) => match probe_log_dir(&paths) {
            Ok(()) => targets.push(Target::new(TargetKind::Folder {
                path: paths.logs_dir(),
                file_name: Some(LOG_FILE_NAME.to_owned()),
            })),
            Err(reason) => {
                let _ = DEGRADED.set(reason);
            }
        },
        None => {
            let _ = DEGRADED.set("home directory unavailable".to_owned());
        }
    }

    builder.plugin(
        LogBuilder::new()
            .level(level)
            .max_file_size(MAX_FILE_SIZE)
            .rotation_strategy(RotationStrategy::KeepSome(KEEP_ARCHIVES))
            .file_open_strategy(FileOpenStrategy::Append)
            .targets(targets)
            .build(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_level_accepts_bare_level_names() {
        assert_eq!(parse_level("error"), Some(LevelFilter::Error));
        assert_eq!(parse_level("warn"), Some(LevelFilter::Warn));
        assert_eq!(parse_level("info"), Some(LevelFilter::Info));
        assert_eq!(parse_level("debug"), Some(LevelFilter::Debug));
        assert_eq!(parse_level("trace"), Some(LevelFilter::Trace));
    }

    #[test]
    fn parse_level_is_case_insensitive_and_tolerates_surrounding_space() {
        assert_eq!(parse_level(" DEBUG "), Some(LevelFilter::Debug));
        assert_eq!(parse_level("Trace"), Some(LevelFilter::Trace));
    }

    #[test]
    fn parse_level_rejects_garbage_and_filter_directives() {
        assert_eq!(parse_level(""), None);
        assert_eq!(parse_level("verbose"), None);
        // EnvFilter 指令语法不支持：假装支持只会静默误解析。
        assert_eq!(parse_level("agent_maestro_lib=debug"), None);
    }

    /// 捕获窗口按线程隔离：另一线程开自己的窗口既不进本窗口，也清空不了它。
    /// 这条把「窗口断言不可被并发测试弄空/弄脏」钉成可执行断言。
    #[test]
    fn capture_windows_are_isolated_per_thread() {
        let window = capture::capture_window();
        log::debug!("capture probe: main");
        let other = std::thread::spawn(|| {
            let window = capture::capture_window();
            log::debug!("capture probe: spawned");
            window.lines()
        })
        .join()
        .unwrap();

        let main = window.lines();
        assert!(
            main.iter()
                .any(|line| line.ends_with("capture probe: main")),
            "本线程窗口捕获本线程日志：{main:?}"
        );
        assert!(
            !main
                .iter()
                .any(|line| line.ends_with("capture probe: spawned")),
            "其它线程的日志不落进本窗口：{main:?}"
        );
        assert!(
            other
                .iter()
                .any(|line| line.ends_with("capture probe: spawned")),
            "另一线程的窗口捕获它自己的日志：{other:?}"
        );
        assert!(
            !other
                .iter()
                .any(|line| line.ends_with("capture probe: main")),
            "另一线程的窗口不受本线程影响：{other:?}"
        );
    }
}
