use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

/// 插件 metadata 的唯一来源：插件根目录的 `manifest.json`。
///
/// 宿主直接读文件，组件不导出 get-metadata；容忍未知字段，
/// 必填字段缺失或插件 id 不合法即加载失败（见 issue #34）。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    /// 插件 id：`[a-z][a-z0-9-_]*`，同时是插件安装目录名。
    pub id: String,
    pub name: String,
    /// 适配的工具（如 `pi`），仅用于展示。
    pub tool: String,
    /// 插件被授权写入的配置目录；`~` 前缀在装载时展开。
    pub config_dir: String,
    /// 入口 wasm 文件，相对插件根目录。
    pub entry: String,
}

/// 解析并校验 manifest 文本。
pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let manifest: Manifest =
        serde_json::from_str(text).map_err(|e| format!("manifest.json 不合法：{e}"))?;
    if !is_valid_plugin_id(&manifest.id) {
        return Err(format!(
            "manifest.json 不合法：插件 id「{}」需以小写字母开头，仅允许小写字母、数字、连字符和下划线",
            manifest.id
        ));
    }
    for (field, value) in [
        ("name", &manifest.name),
        ("tool", &manifest.tool),
        ("config_dir", &manifest.config_dir),
        ("entry", &manifest.entry),
    ] {
        if value.is_empty() {
            return Err(format!("manifest.json 不合法：{field} 不能为空"));
        }
    }
    // entry 必须是插件目录内的相对路径：绝对路径或含 `..` 上跳即拒绝，
    // 防止外置 manifest 借 entry 读取插件根目录之外的文件（读取前的最终
    // 规范化校验见 `resolve_entry`）。
    let entry_path = Path::new(&manifest.entry);
    if entry_path.is_absolute() || entry_path.components().any(|c| c == Component::ParentDir) {
        return Err(format!(
            "manifest.json 不合法：entry「{}」必须是插件目录内的相对路径",
            manifest.entry
        ));
    }
    Ok(manifest)
}

/// 把 manifest.entry 解析为插件根目录内的安全绝对路径。
///
/// 先规范化插件根目录与拼接结果，再验证解析路径仍在根目录内：
/// 指向根目录之外的符号链接同样被拒绝。
pub fn resolve_entry(root: &Path, entry: &str) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("插件目录不可访问：{e}"))?;
    let target = root.join(Path::new(entry));
    let resolved = target
        .canonicalize()
        .map_err(|_| format!("插件入口文件缺失：{}", target.display()))?;
    if !resolved.starts_with(&root) {
        return Err(format!(
            "manifest.json 不合法：entry「{entry}」逃逸了插件目录"
        ));
    }
    Ok(resolved)
}

/// 插件 id 规则与 Provider slug 一致（见 CONTEXT.md）。
pub fn is_valid_plugin_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn manifest_parses_required_fields_and_tolerates_unknown_fields() {
        let manifest = parse_manifest(
            r#"{
                "id": "pi",
                "name": "Pi",
                "tool": "pi",
                "config_dir": "~/.pi",
                "entry": "plugin.wasm",
                "author": "someone",
                "version": "0.2.0"
            }"#,
        )
        .unwrap();

        assert_eq!(manifest.id, "pi");
        assert_eq!(manifest.name, "Pi");
        assert_eq!(manifest.tool, "pi");
        assert_eq!(manifest.config_dir, "~/.pi");
        assert_eq!(manifest.entry, "plugin.wasm");
    }

    #[test]
    fn manifest_missing_required_field_is_rejected() {
        // 只留 id：其余必填缺失即加载失败。
        let err = parse_manifest(r#"{"id": "pi"}"#).unwrap_err();
        assert!(err.contains("manifest.json 不合法"), "{err}");
    }

    #[test]
    fn manifest_with_invalid_id_is_rejected() {
        for id in ["Pi", "1pi", "-pi", "pi.", "pi 中文", ""] {
            let text = format!(
                r#"{{ "id": "{id}", "name": "x", "tool": "x", "config_dir": "~/.x", "entry": "p.wasm" }}"#
            );
            let err = parse_manifest(&text).unwrap_err();
            assert!(err.contains("id"), "id {id:?} 应被拒绝：{err}");
        }
    }

    #[test]
    fn valid_plugin_ids_are_accepted() {
        assert!(is_valid_plugin_id("pi"));
        assert!(is_valid_plugin_id("a1"));
        assert!(is_valid_plugin_id("some-plugin_2"));
        assert!(!is_valid_plugin_id(""));
        assert!(!is_valid_plugin_id("Pi"));
        assert!(!is_valid_plugin_id("1pi"));
        assert!(!is_valid_plugin_id("_pi"));
        assert!(!is_valid_plugin_id("pi."));
        assert!(!is_valid_plugin_id("pi x"));
    }

    #[test]
    fn manifest_with_escaping_entry_is_rejected() {
        for entry in ["/etc/passwd", "../outside.wasm", "sub/../../outside.wasm"] {
            let text = format!(
                r#"{{ "id": "pi", "name": "x", "tool": "x", "config_dir": "~/.x", "entry": "{entry}" }}"#
            );
            let err = parse_manifest(&text).unwrap_err();
            assert!(err.contains("entry"), "entry {entry:?} 应被拒绝：{err}");
        }
    }

    #[test]
    fn resolve_entry_rejects_symlink_escape_and_missing_file() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("secret.wasm");
        fs::write(&outside_file, b"x").unwrap();
        std::os::unix::fs::symlink(&outside_file, root.path().join("escape.wasm")).unwrap();

        let err = resolve_entry(root.path(), "escape.wasm").unwrap_err();
        assert!(err.contains("逃逸"), "{err}");
        assert!(resolve_entry(root.path(), "missing.wasm").is_err());

        let real = root.path().join("plugin.wasm");
        fs::write(&real, b"x").unwrap();
        assert_eq!(resolve_entry(root.path(), "plugin.wasm").unwrap(), real);
    }
}
