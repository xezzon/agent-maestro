//! 原子落盘：先写同目录临时文件并落盘，再 rename 覆盖目标，避免半截文件。
//!
//! 「怎么写」收在 crate 级 [`crate::fs_util`]：宿主侧另有一处同样的
//! 「tmp + rename」（插件落位产物），两份实现各自漂移过一次——本模块只决定
//! 「写什么」与「要不要 fsync」，怎么做由 `fs_util` 统一。

use std::{fs, path::Path};

use crate::config::Config;
use crate::errors::ConfigError;
use crate::fs_util::{Durability, write_atomic};

/// 原子写入：同目录临时文件 → 落盘 → rename 覆盖目标。
///
/// 任何一步失败都不动目标文件：要么还是旧内容，要么完全没写。
pub(super) fn persist(config_path: &Path, config: &Config) -> Result<(), ConfigError> {
    let dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    let io_error = |e: std::io::Error| ConfigError::io(config_path, e.to_string());
    fs::create_dir_all(dir).map_err(io_error)?;

    let json =
        serde_json::to_string_pretty(config).map_err(|e| ConfigError::serialize(e.to_string()))?;

    // 配置文件没有第二来源：落盘前 fsync，保证文件内容已到设备——掉电后不会
    // 读到半截内容。rename 的目录项未 fsync，掉电后目标可能仍是旧版（首次写入
    // 时可能整个缺失）：耐久性只到文件内容为止，见 [`Durability`]。
    write_atomic(
        config_path,
        format!("{json}\n").as_bytes(),
        Durability::Synced,
    )
    .map_err(io_error)
}
