//! 原子落盘：先写同目录临时文件并落盘，再 rename 覆盖目标，避免半截文件。
//!
//! 「怎么写」收在 crate 级 [`crate::fs_util`]：宿主侧另有一处同样的
//! 「tmp + rename」（插件落位产物），两份实现各自漂移过一次——本模块只决定
//! 「写什么」与「要不要 fsync」，怎么做由 `fs_util` 统一。

use std::{fs, path::Path};

use super::schema::{Config, StoreError};
use crate::fs_util::{Durability, write_atomic};

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

    // 配置文件没有第二来源：落盘前 fsync，掉电后仍是新内容。
    write_atomic(
        config_path,
        format!("{json}\n").as_bytes(),
        Durability::Synced,
    )
    .map_err(io_error)
}
