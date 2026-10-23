//! 测试期日志捕获（L2 金丝雀用）：只捕获本 crate 的日志，且缓冲封顶。
//!
//! **按 target 过滤，而不是抬高日志级别。** `log` 的级别与 logger 都是进程级、
//! 一次性、不可移除的：抬到 `Trace` 会连带打开 wasmtime / cranelift 的逐指令
//! 日志（`vcode::emit` 每条机器指令一行、egraph 每指令数行、`timing` 每个 pass
//! 一行），单次组件编译即数十 MB；测试进程里几十处编译累计到 GB 级并被永久
//! 保留，足以把整机压进 swap。金丝雀要证的是「本 crate 的日志语句不泄露
//! api_key」——api_key 只经 WIT 结构进 sandbox，wasmtime / wiggle 不打印 guest
//! 数据——因此按 target 过滤不损失断言强度。别把这里「简化」成抬高全局级别。
//!
//! 捕获窗口按**线程**隔离（见 [`capture_window`]）：窗口只收本线程在窗口存续
//! 期间发出的日志，因此并发的捕获测试各有各的窗口（互不清空、互不污染），
//! 窗口外测试的日志也落不进任何窗口；不需要全局缓冲，也就没有「清空与读取
//! 之间被别人插一杠」的窗口。
//!
//! 本模块只服务测试：生产代码不需要它，故与 [`super`] 的生产设施分开成
//! 独立文件——`logging/mod.rs` 一半是生产设施、一半是测试设施时，两边都在
//! 对方的视线里。

use std::{cell::RefCell, rc::Rc, sync::OnceLock};

use log::{LevelFilter, Log, Metadata, Record};

/// 本 crate 的日志 target 前缀（`module_path!()` 以 crate 根名开头）。
/// 用 `CARGO_CRATE_NAME` 而非字面量：lib target 改名时不会静默失配。
const CRATE_TARGET: &str = env!("CARGO_CRATE_NAME");
/// 缓冲行数硬上限：任何未来的刷屏路径都不会把测试进程的内存吃光。
/// 本 crate 全部测试的日志量远小于该值，正常不会触及。
const MAX_LINES: usize = 8 * 1024;

static INSTALLED: OnceLock<()> = OnceLock::new();

thread_local! {
    /// 本线程当前的活动捕获窗口；`None` 即本线程不在窗口内，日志被丢弃。
    static WINDOW: RefCell<Option<Rc<RefCell<Vec<String>>>>> = const { RefCell::new(None) };
}

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
        // 只收本线程窗口内的记录：窗口外的输出（含并发测试）不落进任何窗口。
        let Ok(window) = WINDOW.try_with(|slot| slot.borrow().clone()) else {
            return;
        };
        let Some(window) = window else {
            return;
        };
        let mut lines = window.borrow_mut();
        if lines.len() < MAX_LINES {
            lines.push(format!(
                "[{}][{}] {}",
                record.level(),
                record.target(),
                record.args()
            ));
        }
    }

    fn flush(&self) {}
}

/// 打开一个捕获窗口：安装进程内捕获 logger（仅安装一次，重复调用复用），
/// 并把本线程标记为在窗口内，返回该窗口。窗口随 [`CaptureWindow`] 一起丢弃。
pub(crate) fn capture_window() -> CaptureWindow {
    INSTALLED.get_or_init(|| {
        let _ = log::set_boxed_logger(Box::new(Capture));
        // `Debug` 足够：本 crate 最低只用 `debug!`，`Trace` 只会放进第三方逐指令日志。
        log::set_max_level(LevelFilter::Debug);
    });
    let lines = Rc::new(RefCell::new(Vec::new()));
    WINDOW.with(|slot| *slot.borrow_mut() = Some(Rc::clone(&lines)));
    CaptureWindow { lines }
}

/// 一次捕获测试的窗口；存续期间本线程发出的本 crate 日志记入 [`Self::lines`]。
///
/// 窗口只持有缓冲的所有权、不持有它的借用：捕获 logger 与投影跑在同一线程，
/// 每条记录只短暂借出一次。窗口若把借用占满整个投影，投影途中同一线程上的
/// 捕获 logger 就会撞上已借出的 `RefCell`。
pub(crate) struct CaptureWindow {
    lines: Rc<RefCell<Vec<String>>>,
}

impl CaptureWindow {
    /// 窗口内捕获到的行，格式 `[LEVEL][target] 消息`。
    pub(crate) fn lines(&self) -> Vec<String> {
        self.lines.borrow().clone()
    }
}

impl Drop for CaptureWindow {
    fn drop(&mut self) {
        // 关闭本线程窗口；TLS 已析构时无从清理，忽略即可。
        let _ = WINDOW.try_with(|slot| *slot.borrow_mut() = None);
    }
}
