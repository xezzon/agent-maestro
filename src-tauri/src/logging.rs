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

use crate::paths::MaestroPaths;

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
pub fn degraded_reason() -> Option<&'static str> {
    DEGRADED.get().map(String::as_str)
}

/// 环境变量里的非法级别值；由应用 setup 阶段读取并打 WARN。
pub fn invalid_level() -> Option<&'static str> {
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
fn probe_log_dir(paths: &MaestroPaths) -> Result<(), String> {
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
pub fn attach(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
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
    match MaestroPaths::try_get() {
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

/// 测试期日志捕获（L2 金丝雀用）：只捕获本 crate 的日志，且缓冲封顶。
///
/// `log` 的级别与 logger 都是进程级、一次性、不可移除的：抬到 `Trace` 会连带打开
/// wasmtime / cranelift 的逐指令日志（`vcode::emit` 每条机器指令一行、egraph 每指令
/// 数行、`timing` 每个 pass 一行），单次组件编译即数十 MB；测试进程里几十处编译
/// 累计到 GB 级并被永久保留，足以把整机压进 swap。金丝雀要证的是「本 crate 的
/// 日志语句不泄露 api_key」——api_key 只经 WIT 结构进 sandbox，wasmtime / wiggle
/// 不打印 guest 数据——因此按 target 过滤不损失断言强度。
#[cfg(test)]
pub(crate) mod capture {
    use log::{LevelFilter, Log, Metadata, Record};
    use std::sync::{Mutex, OnceLock};

    /// 本 crate 的日志 target 前缀（`module_path!()` 以 crate 根名开头）。
    /// 用 `CARGO_CRATE_NAME` 而非字面量：lib target 改名时不会静默失配。
    const CRATE_TARGET: &str = env!("CARGO_CRATE_NAME");
    /// 缓冲行数硬上限：任何未来的刷屏路径都不会把测试进程的内存吃光。
    /// 本 crate 全部测试的日志量远小于该值，正常不会触及。
    const MAX_LINES: usize = 8 * 1024;

    static BUFFER: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

    struct Capture;

    impl Log for Capture {
        fn enabled(&self, _metadata: &Metadata) -> bool {
            true
        }

        fn log(&self, record: &Record) {
            // 先判 target 再 format：被丢弃的记录不付出格式化与分配代价。
            if !record.target().starts_with(CRATE_TARGET) {
                return;
            }
            if let Some(buffer) = BUFFER.get() {
                let mut buffer = buffer.lock().unwrap();
                if buffer.len() < MAX_LINES {
                    buffer.push(format!(
                        "[{}][{}] {}",
                        record.level(),
                        record.target(),
                        record.args()
                    ));
                }
            }
        }

        fn flush(&self) {}
    }

    /// 安装进程内捕获 logger（仅安装一次，重复调用复用）并清空缓冲，返回共享缓冲。
    /// 清空而非累积：金丝雀断言只关心本次测试窗口内的输出，并发测试的日志也会被
    /// 限制在各自的窗口上。
    pub(crate) fn captured_logs() -> &'static Mutex<Vec<String>> {
        let buffer = BUFFER.get_or_init(|| {
            let _ = log::set_boxed_logger(Box::new(Capture));
            // `Debug` 足够：本 crate 最低只用 `debug!`，`Trace` 只会放进第三方逐指令日志。
            log::set_max_level(LevelFilter::Debug);
            Mutex::new(Vec::new())
        });
        buffer.lock().unwrap().clear();
        buffer
    }
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
}
