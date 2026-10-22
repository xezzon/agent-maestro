use std::path::{Path, PathBuf};

use crate::test_global::Global;

#[cfg(test)]
use crate::test_global::TestGuard;

/// 全局主目录：生产期进程内一份，测试期每线程一份（见 [`crate::test_global`]）。
static MAESTRO_PATHS: Global<MaestroPaths> = Global::new();

#[derive(Clone)]
pub(crate) struct MaestroPaths {
    home: PathBuf,
}

impl MaestroPaths {
    fn new(home: &Path) -> Self {
        Self {
            home: home.to_path_buf(),
        }
    }

    /// 生产环境初始化全局实例；重复调用无效（以首次为准）。
    ///
    /// 必须在第一次调用 [`MaestroPaths::get`] 之前完成，否则 `get` 会 panic。
    pub(crate) fn init(home: &Path) {
        MAESTRO_PATHS.set(Self::new(home));
    }

    /// 获取全局实例（值语义，clone 成本极低）。
    ///
    /// 尚未初始化时 panic；调用方若无法保证初始化顺序，用 [`MaestroPaths::try_get`]。
    pub(crate) fn get() -> Self {
        Self::try_get().expect("MaestroPaths 尚未初始化：调用 MaestroPaths::init 后再使用")
    }

    /// 尝试获取全局实例；尚未初始化时返回 `None`。
    ///
    /// 用于启动早期（logger attach 等）调用点——此时主目录可能还没解析出来。
    pub(crate) fn try_get() -> Option<Self> {
        MAESTRO_PATHS.get()
    }

    /// 测试辅助：设置当前线程的全局实例，返回 RAII guard（drop 时自动清理）。
    #[cfg(test)]
    fn test_guard(home: &Path) -> TestGuard<MaestroPaths> {
        Self::init(home);
        TestGuard::new(&MAESTRO_PATHS)
    }

    pub(crate) fn maestro_dir(&self) -> PathBuf {
        self.home.join(".maestro")
    }

    pub(crate) fn config_path(&self) -> PathBuf {
        self.maestro_dir().join("config.json")
    }

    pub(crate) fn plugins_dir(&self) -> PathBuf {
        self.maestro_dir().join("plugins")
    }

    /// 日志目录（ADR 0008）：`~/.maestro/logs`，用户可见、易备份、跨平台统一。
    pub(crate) fn logs_dir(&self) -> PathBuf {
        self.maestro_dir().join("logs")
    }

    pub(crate) fn plugin_dir(&self, plugin_id: &str) -> PathBuf {
        self.plugins_dir().join(plugin_id)
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
        assert!(MAESTRO_PATHS.get().is_none());
    }
}
