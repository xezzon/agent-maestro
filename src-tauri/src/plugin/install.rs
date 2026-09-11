//! 第三方插件的落位目录布局：`<home>/.maestro/plugins/<id>/{manifest.json, plugin.wasm}`。
//!
//! 落位 manifest 与上游 manifest 仅 `entry` 一处不同（重写为 [`PLACED_WASM`]）。
//! 应用启动只读该目录重建注册表，因此 https 插件离线可用（见 ADR 0006）。

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use super::manifest::{self, Manifest, SourceKind};

/// 落位目录中的 manifest 文件名。
pub const PLACED_MANIFEST: &str = "manifest.json";
/// 落位目录中的 wasm 文件名；上游 manifest 的 `entry` 落位时重写为该名。
pub const PLACED_WASM: &str = "plugin.wasm";

/// 宿主插件根目录（`~/.maestro/plugins`）。
pub fn root(home: &Path) -> PathBuf {
    home.join(".maestro").join("plugins")
}

/// 指定插件 id 的落位目录。
pub fn plugin_dir(root: &Path, id: &str) -> PathBuf {
    root.join(id)
}

/// 落位读取结果：磁盘 manifest 与 wasm 字节。
#[derive(Debug)]
pub struct Placed {
    pub manifest: Manifest,
    pub wasm: Vec<u8>,
}

/// 落位：上游 manifest（`entry` 重写为本地文件名）与 wasm 先写进同目录的临时目录，
/// 再整体替换目标目录。
///
/// 目标已存在时先备份旧目录，替换成功才删除备份、失败则恢复备份——因此更新失败时
/// 旧版本保持可用。临时目录与目标同父目录，保证替换是一次改名而非跨设备拷贝。
pub fn place(target: &Path, upstream_manifest: &str, wasm: &[u8]) -> Result<(), String> {
    let placed_manifest = manifest::rewrite_entry(upstream_manifest, PLACED_WASM)?;
    let parent = target
        .parent()
        .ok_or_else(|| format!("落位目录不合法：{}", target.display()))?;
    fs::create_dir_all(parent).map_err(|e| format!("创建插件目录失败：{e}"))?;

    let staging = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(parent)
        .map_err(|e| format!("创建临时落位目录失败：{e}"))?;
    let staging_path = staging.path().to_owned();
    let io_error = |what: &str| {
        let what = what.to_owned();
        move |e: std::io::Error| format!("写入{what}失败：{e}")
    };
    fs::write(staging_path.join(PLACED_MANIFEST), placed_manifest)
        .map_err(io_error(PLACED_MANIFEST))?;
    fs::write(staging_path.join(PLACED_WASM), wasm).map_err(io_error(PLACED_WASM))?;

    match swap(&staging_path, target) {
        Ok(()) => {
            let _ = staging.keep();
            Ok(())
        }
        Err(e) => {
            // 临时目录仍留在磁盘上，随 TempDir 一并清理。
            Err(e)
        }
    }
}

/// 用 staging 目录替换目标目录；目标不存在即直接改名。
fn swap(staging: &Path, target: &Path) -> Result<(), String> {
    if !target.exists() {
        return fs::rename(staging, target).map_err(|e| format!("落位插件目录失败：{e}"));
    }
    // 插件 id 仅允许 [a-z0-9-_]，故 `<id>.old` 不会与其它插件的落位目录同名。
    let backup = target.with_extension("old");
    if backup.exists() {
        fs::remove_dir_all(&backup).map_err(|e| format!("清理上次落位的备份目录失败：{e}"))?;
    }
    fs::rename(target, &backup).map_err(|e| format!("备份旧版本失败：{e}"))?;
    match fs::rename(staging, target) {
        Ok(()) => {
            // 替换已成功：旧版本删除失败不影响新版本可用。
            let _ = fs::remove_dir_all(&backup);
            Ok(())
        }
        Err(e) => {
            let _ = fs::rename(&backup, target);
            Err(format!("替换插件目录失败：{e}"))
        }
    }
}

/// 读取落位目录：磁盘 manifest 与 wasm 字节。
///
/// 落位目录由宿主全权持有，wasm 固定为 [`PLACED_WASM`]。
pub fn read(target: &Path) -> Result<Placed, String> {
    let manifest_path = target.join(PLACED_MANIFEST);
    let manifest_text = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("读取 {} 失败：{e}", manifest_path.display()))?;
    let manifest = manifest::parse_manifest(SourceKind::File, &manifest_text)?;

    let wasm_path = target.join(PLACED_WASM);
    let wasm =
        fs::read(&wasm_path).map_err(|e| format!("读取 {} 失败：{e}", wasm_path.display()))?;
    Ok(Placed { manifest, wasm })
}

/// 删除落位目录；目录不存在即成功（幂等）。
pub fn remove(target: &Path) -> Result<(), String> {
    match fs::remove_dir_all(target) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("删除插件目录 {} 失败：{e}", target.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{
        "id": "zed",
        "name": "Zed",
        "tool": "zed",
        "config_dir": "~/.config/zed",
        "entry": "https://example.com/releases/download/v1/plugin.wasm",
        "author": "someone"
    }"#;

    fn temp_root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn place_writes_layout_with_rewritten_entry() {
        let root = temp_root();
        let target = plugin_dir(root.path(), "zed");

        place(&target, MANIFEST, b"wasm-bytes").unwrap();

        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(target.join(PLACED_MANIFEST)).unwrap())
                .unwrap();
        assert_eq!(
            manifest["entry"], PLACED_WASM,
            "落位时 entry 重写为本地相对名"
        );
        assert_eq!(manifest["author"], "someone", "上游字段原样保留");
        assert_eq!(fs::read(target.join(PLACED_WASM)).unwrap(), b"wasm-bytes");

        let placed = read(&target).unwrap();
        assert_eq!(placed.manifest.id, "zed");
        assert_eq!(placed.manifest.config_dir, "~/.config/zed");
        assert_eq!(placed.wasm, b"wasm-bytes");
    }

    #[test]
    fn place_replaces_existing_dir_and_drops_backup() {
        let root = temp_root();
        let target = plugin_dir(root.path(), "zed");
        place(&target, MANIFEST, b"old").unwrap();

        place(&target, MANIFEST, b"new").unwrap();

        assert_eq!(fs::read(target.join(PLACED_WASM)).unwrap(), b"new");
        assert!(
            !target.with_extension("old").exists(),
            "替换成功后不得留下备份目录"
        );
        assert!(
            !fs::read_dir(root.path()).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".staging-")),
            "替换成功后不得留下临时落位目录"
        );
    }

    #[test]
    fn place_rejects_manifest_that_is_not_an_object() {
        let root = temp_root();
        let target = plugin_dir(root.path(), "zed");

        let err = place(&target, "[1, 2]", b"wasm").unwrap_err();

        assert!(err.contains("manifest.json"), "{err}");
        assert!(!target.exists(), "校验失败不得落位");
    }

    #[test]
    fn remove_is_idempotent() {
        let root = temp_root();
        let target = plugin_dir(root.path(), "zed");
        place(&target, MANIFEST, b"wasm").unwrap();

        remove(&target).unwrap();
        assert!(!target.exists());
        remove(&target).unwrap();
    }

    #[test]
    fn read_reports_missing_files() {
        let root = temp_root();
        let target = plugin_dir(root.path(), "zed");

        assert!(read(&target).unwrap_err().contains("manifest.json"));

        fs::create_dir_all(&target).unwrap();
        fs::write(target.join(PLACED_MANIFEST), MANIFEST).unwrap();
        assert!(read(&target).unwrap_err().contains(PLACED_WASM));
    }
}
