//! 原子落盘：同目录临时文件 + rename 覆盖目标。
//!
//! 宿主侧此前有两处「tmp + rename」：配置存储（`store::persist`）与插件落位
//! 产物（`plugin::write_placed_file`）。两份实现各写一遍，一份落盘前 fsync、
//! 一份不 fsync，且没有任何一句话说明为什么不同——将来若要求插件产物也抗
//! 掉电，必须记得改第二处。这里收成一处：**怎么做**由本模块统一，**要不要
//! fsync** 成为调用点显式声明的策略。

use std::{
    io::{self, Write},
    path::Path,
};

/// 落盘耐久性：rename 之前是否把内容 fsync 到设备。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Durability {
    /// 落盘前 fsync：文件内容已到设备，掉电后不会读到半截内容。
    ///
    /// 用于配置文件：它没有第二来源，丢了就是用户的全部配置。注意这只覆盖
    /// **文件内容**——rename 本身的目录项没有 fsync，掉电后目标可能仍是旧
    /// 那一版（首次写入时可能整个不存在），但不会是写坏的一半。
    Synced,
    /// 不 fsync：rename 仍是原子的（不会留下半截文件），但内容可能还在页缓存里。
    ///
    /// 用于插件侧产物（manifest / wasm / 插件配置）：wasm 与 manifest 可从来源
    /// 重新获取，插件配置随插件存在、重装即从干净状态开始（见 ADR 0018）——
    /// 每次安装与重载都 fsync 一遍的代价大于收益。
    Unsynced,
}

/// 原子写入：同目录临时文件 → 写入 → （按耐久性策略）fsync → rename 覆盖目标。
///
/// 任何一步失败都不动目标文件：要么还是旧内容，要么完全没写。
///
/// 目标所在的目录必须已存在（建目录是调用方的事：插件目录由落位流程建，
/// 配置目录由存储层建）。临时文件与目标同目录，故 rename 不跨文件系统、
/// 是原子的。
///
/// 原子性覆盖的是**内容**：rename 之后无论是否 fsync，都不会留下写坏的一半。
/// 耐久性到此为止——rename 的目录项本身没有 fsync，掉电后目标可能仍是旧版本
/// （首次写入时可能整个不存在）。要更强的保证得在 rename 之后补一次父目录
/// fsync，本模块尚未这么做。
pub(crate) fn write_atomic(
    target: &Path,
    contents: &[u8],
    durability: Durability,
) -> io::Result<()> {
    let dir = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| target.to_string_lossy().into_owned());

    let mut tmp = tempfile::Builder::new()
        .prefix(&format!(".{name}.tmp-"))
        .tempfile_in(dir)
        .map_err(|e| io::Error::new(e.kind(), format!("创建 {name} 的临时文件失败：{e}")))?;
    tmp.write_all(contents)
        .map_err(|e| io::Error::new(e.kind(), format!("写入 {name} 失败：{e}")))?;
    if durability == Durability::Synced {
        tmp.as_file()
            .sync_all()
            .map_err(|e| io::Error::new(e.kind(), format!("落盘 {name} 失败：{e}")))?;
    }
    // PersistError 内含真正的 io::Error；临时文件随后 drop 时自动清理。
    tmp.persist(target)
        .map_err(|e| io::Error::new(e.error.kind(), format!("覆盖 {name} 失败：{}", e.error)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn write_atomic_replaces_the_target_and_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.json");
        fs::write(&target, b"old").unwrap();

        write_atomic(&target, b"new", Durability::Synced).unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"new");
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name() != "config.json")
            .collect();
        assert!(leftovers.is_empty(), "临时文件应被清理：{leftovers:?}");
    }

    /// 目标不存在时不得留下半截文件：写失败（此处是目录不存在）时目标原样缺席。
    #[test]
    fn write_atomic_fails_cleanly_when_the_directory_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("missing").join("config.json");

        let err = write_atomic(&target, b"new", Durability::Unsynced).unwrap_err();

        assert!(err.to_string().contains("config.json"), "{err}");
        assert!(!target.exists());
    }
}
