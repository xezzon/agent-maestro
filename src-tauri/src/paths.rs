use std::path::{Path, PathBuf};

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

    pub fn home(&self) -> PathBuf {
        self.home.clone()
    }
}
