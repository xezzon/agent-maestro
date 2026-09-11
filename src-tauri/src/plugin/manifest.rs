use std::path::{Component, Path};

use serde::Deserialize;

/// 插件来源种类：决定 `entry` 的校验规则（见 ADR 0006）。
///
/// 三种来源共用同一份 manifest 校验，`entry` 语义按来源分列，
/// 不做跨协议的统一解析。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// 内置于应用：wasm 内嵌，`entry` 不参与解析。
    Builtin,
    /// https 来源：指向正式发布的 manifest，`entry` 必须是 https URL。
    Https,
    /// 本机目录中的 manifest（第三方来源的落位目录），`entry` 是目录内的相对路径。
    File,
}

/// 插件 metadata 的唯一来源：插件根目录的 `manifest.json`。
///
/// 宿主直接读文件，组件不导出 get-metadata；容忍未知字段，
/// 必填字段缺失或插件 id 不合法即加载失败（见 issue #34）。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    /// 插件 id：`[a-z][a-z0-9-_]*`。
    pub id: String,
    pub name: String,
    /// 适配的工具（如 `pi`），仅用于展示。
    pub tool: String,
    /// 插件被授权写入的配置目录；`~` 前缀在装载时展开。
    pub config_dir: String,
    /// 入口 wasm 文件，相对插件根目录。
    pub entry: String,
}

/// 解析并校验 manifest 文本；`entry` 的约束按来源种类分列。
pub fn parse_manifest(kind: SourceKind, text: &str) -> Result<Manifest, String> {
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
    match kind {
        // wasm 内嵌于二进制，entry 不参与解析。
        SourceKind::Builtin => {}
        SourceKind::Https => {
            if !is_https_url(&manifest.entry) {
                return Err(format!(
                    "manifest.json 不合法：https 来源的 entry「{}」必须是 https URL",
                    manifest.entry
                ));
            }
        }
        // entry 必须是 manifest 所在目录内的相对路径：绝对路径或含 `..` 上跳即拒绝，
        // 防止外置 manifest 借 entry 读取插件根目录之外的文件。
        SourceKind::File => {
            let entry_path = Path::new(&manifest.entry);
            if entry_path.is_absolute()
                || entry_path.components().any(|c| c == Component::ParentDir)
            {
                return Err(format!(
                    "manifest.json 不合法：entry「{}」必须是插件目录内的相对路径",
                    manifest.entry
                ));
            }
        }
    }
    Ok(manifest)
}

/// 是否为绝对 https URL：明文 http、其他 scheme 与空主机一律拒绝。
pub fn is_https_url(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|parsed| parsed.scheme() == "https" && parsed.has_host())
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

    fn manifest_text(id: &str, entry: &str) -> String {
        format!(
            r#"{{ "id": "{id}", "name": "x", "tool": "x", "config_dir": "~/.x", "entry": "{entry}" }}"#
        )
    }

    #[test]
    fn manifest_parses_required_fields_and_tolerates_unknown_fields() {
        let manifest = parse_manifest(
            SourceKind::File,
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
        let err = parse_manifest(SourceKind::File, r#"{"id": "pi"}"#).unwrap_err();
        assert!(err.contains("manifest.json 不合法"), "{err}");
    }

    #[test]
    fn manifest_with_invalid_id_is_rejected() {
        for id in ["Pi", "1pi", "-pi", "pi.", "pi 中文", ""] {
            let text = manifest_text(id, "p.wasm");
            let err = parse_manifest(SourceKind::File, &text).unwrap_err();
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
    fn file_entry_must_be_a_relative_path_without_parent_dir() {
        for entry in ["/etc/passwd", "../outside.wasm", "sub/../../outside.wasm"] {
            let err = parse_manifest(SourceKind::File, &manifest_text("pi", entry)).unwrap_err();
            assert!(err.contains("entry"), "entry {entry:?} 应被拒绝：{err}");
        }
        assert!(parse_manifest(SourceKind::File, &manifest_text("pi", "sub/p.wasm")).is_ok());
    }

    #[test]
    fn https_entry_must_be_an_https_url() {
        let ok = manifest_text("pi", "https://example.com/releases/download/v1/plugin.wasm");
        assert_eq!(
            parse_manifest(SourceKind::Https, &ok).unwrap().entry,
            "https://example.com/releases/download/v1/plugin.wasm"
        );

        for entry in [
            "plugin.wasm",
            "sub/p.wasm",
            "/etc/passwd",
            "../outside.wasm",
            "http://example.com/plugin.wasm",
            "https://",
        ] {
            let err = parse_manifest(SourceKind::Https, &manifest_text("pi", entry)).unwrap_err();
            assert!(
                err.contains("https URL"),
                "https 来源的 entry {entry:?} 应被拒绝：{err}"
            );
        }
    }

    #[test]
    fn builtin_entry_is_not_constrained() {
        // 内置插件的 wasm 内嵌于二进制，entry 不参与解析。
        let text = manifest_text("pi", "target/wasm32-wasip2/release/maestro_plugin_pi.wasm");
        assert!(parse_manifest(SourceKind::Builtin, &text).is_ok());
    }

    #[test]
    fn is_https_url_accepts_only_absolute_https_urls() {
        assert!(is_https_url("https://example.com/manifest.json"));
        assert!(is_https_url("https://example.com:8443/a/b?c=d"));
        assert!(!is_https_url("http://example.com/manifest.json"));
        assert!(!is_https_url("file:///tmp/manifest.json"));
        assert!(!is_https_url("/tmp/manifest.json"));
        assert!(!is_https_url("https://"));
        assert!(!is_https_url(""));
    }
}
