//! 原子落盘：先写同目录临时文件并落盘，再 rename 覆盖目标，避免半截文件。

use std::{fs, io::Write, path::Path};

use super::schema::{Config, StoreError};

/// 原子写入：同目录临时文件 → 落盘 → rename 覆盖目标。
///
/// 任何一步失败都不动目标文件：要么还是旧内容，要么完全没写。
pub(super) fn persist(config_path: &Path, config: &Config) -> Result<(), StoreError> {
    let dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    let io_error = |e: std::io::Error| StoreError::Io {
        path: config_path.to_path_buf(),
        detail: e.to_string(),
    };
    fs::create_dir_all(dir).map_err(io_error)?;

    let json = serde_json::to_string_pretty(config).map_err(|e| StoreError::Serialize {
        detail: e.to_string(),
    })?;

    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(io_error)?;
    tmp.write_all(format!("{json}\n").as_bytes())
        .map_err(io_error)?;
    tmp.as_file().sync_all().map_err(io_error)?;

    // PersistError 内含真正的 io::Error；临时文件随后 drop 时自动清理。
    tmp.persist(config_path).map_err(|e| io_error(e.error))?;
    Ok(())
}
