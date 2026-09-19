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

    pub fn plugin_dir(&self, plugin_id: &str) -> PathBuf {
        self.plugins_dir().join(plugin_id)
    }

    pub fn home(&self) -> PathBuf {
        self.home.clone()
    }
}
