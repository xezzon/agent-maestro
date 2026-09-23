use std::path::{Path, PathBuf};

#[cfg(not(test))]
static MAESTRO_PATHS: std::sync::OnceLock<MaestroPaths> = std::sync::OnceLock::new();

#[cfg(test)]
thread_local! {
    static MAESTRO_PATHS_TEST: std::cell::RefCell<Option<MaestroPaths>> =
        const { std::cell::RefCell::new(None) };
}

#[derive(Clone)]
pub struct MaestroPaths {
    home: PathBuf,
}

impl MaestroPaths {
    pub fn new(home: &Path) -> Self {
        Self {
            home: home.to_path_buf(),
        }
    }

    /// 生产环境初始化全局实例；重复调用无效（以首次为准）。
    ///
    /// 必须在第一次调用 [`MaestroPaths::get`] 之前完成，否则 `get` 会 panic。
    pub fn init(home: &Path) {
        #[cfg(not(test))]
        {
            let _ = MAESTRO_PATHS.set(Self::new(home));
        }
        #[cfg(test)]
        {
            MAESTRO_PATHS_TEST.with(|cell| {
                *cell.borrow_mut() = Some(Self::new(home));
            });
        }
    }

    /// 获取全局实例（值语义，clone 成本极低）。
    ///
    /// 尚未初始化时 panic；调用方若无法保证初始化顺序，用 [`MaestroPaths::try_get`]。
    pub fn get() -> Self {
        Self::try_get().expect("MaestroPaths 尚未初始化：调用 MaestroPaths::init 后再使用")
    }

    /// 尝试获取全局实例；尚未初始化时返回 `None`。
    ///
    /// 用于启动早期（logger attach 等）调用点——此时主目录可能还没解析出来。
    pub fn try_get() -> Option<Self> {
        #[cfg(not(test))]
        {
            MAESTRO_PATHS.get().cloned()
        }
        #[cfg(test)]
        {
            MAESTRO_PATHS_TEST.with(|cell| cell.borrow().clone())
        }
    }

    /// 测试辅助：设置当前线程的全局实例，返回 RAII guard（drop 时自动清理）。
    #[cfg(test)]
    pub fn test_guard(home: &Path) -> TestGuard {
        Self::init(home);
        TestGuard
    }

    pub fn maestro_dir(&self) -> PathBuf {
        self.home.join(".maestro")
    }

    pub fn config_path(&self) -> PathBuf {
        self.maestro_dir().join("config.json")
    }

    pub fn plugins_dir(&self) -> PathBuf {
        self.maestro_dir().join("plugins")
    }

    /// 日志目录（ADR 0008）：`~/.maestro/logs`，用户可见、易备份、跨平台统一。
    pub fn logs_dir(&self) -> PathBuf {
        self.maestro_dir().join("logs")
    }

    pub fn plugin_dir(&self, plugin_id: &str) -> PathBuf {
        self.plugins_dir().join(plugin_id)
    }
}

/// 测试 RAII guard：drop 时清理当前线程的 [`MaestroPaths`] 全局实例。
#[cfg(test)]
#[must_use = "guard 被立即 drop 将立即清理全局实例"]
pub struct TestGuard;

#[cfg(test)]
impl Drop for TestGuard {
    fn drop(&mut self) {
        MAESTRO_PATHS_TEST.with(|cell| {
            *cell.borrow_mut() = None;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_guard_sets_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        {
            let _g = MaestroPaths::test_guard(dir.path());
            let paths = MaestroPaths::get();
            assert_eq!(paths.maestro_dir(), dir.path().join(".maestro"));
            assert_eq!(paths.config_path(), dir.path().join(".maestro/config.json"));
        }
        // guard drop 后应清理
        MAESTRO_PATHS_TEST.with(|cell| {
            assert!(cell.borrow().is_none());
        });
    }
}
