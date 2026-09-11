use crate::plugin::builtin;
use serde::Deserialize;
use std::path::{Component, Path};

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
    /// 本机指向 manifest.json 的绝对路径（本地调试回路）：`entry` 是 manifest
    /// 所在目录内的相对路径或 https URL。
    File,
}

impl SourceKind {
    /// 来源字符串的种类：装载与生命周期操作按此分派。
    ///
    /// `builtin:<id>` 内置于应用；https URL 为正式发布来源；本机绝对路径为 file 来源。
    /// 其余形态（如旧配置残留的 Git 地址、相对路径）无法识别，返回 `None` 由调用方
    /// 报「暂不支持的插件来源」。
    pub fn from_source(source: &str) -> Option<Self> {
        if source.starts_with(builtin::SOURCE_PREFIX) {
            Some(SourceKind::Builtin)
        } else if is_https_url(source) {
            Some(SourceKind::Https)
        } else if Path::new(source).is_absolute() {
            Some(SourceKind::File)
        } else {
            None
        }
    }
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
    /// 入口 wasm 的回源地址：https URL，或相对 `manifest.json` 所在目录的相对路径
    /// （约束按来源种类分列，见 ADR 0006；内置插件的 wasm 内嵌，本字段不参与解析）。
    pub entry: String,
}

/// 解析并校验 manifest 文本；`entry` 的约束按来源种类分列。
pub fn parse_manifest(kind: SourceKind, text: &str) -> Result<Manifest, String> {
    let manifest = parse_metadata(text)?;
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
        // file 来源的 entry 是插件目录内的相对路径或 https URL（回源到网络）：绝对路径、
        // `..` 上跳与其他 scheme（http://、file://…）一律拒绝，防止外置 manifest 借 entry
        // 读取插件根目录之外的文件。
        SourceKind::File => {
            if !is_https_url(&manifest.entry) {
                let entry_path = Path::new(&manifest.entry);
                if entry_path.is_absolute()
                    || entry_path.components().any(|c| c == Component::ParentDir)
                    // 带 scheme 的形态到不了联网获取分支，会在来源目录里读一个不可能
                    // 存在的文件：在校验期拒掉，而不是报误导性的读取错误。
                    || url::Url::parse(&manifest.entry).is_ok()
                {
                    return Err(format!(
                        "manifest.json 不合法：entry「{}」必须是插件目录内的相对路径或 https URL",
                        manifest.entry
                    ));
                }
            }
        }
    }
    Ok(manifest)
}

/// 解析落位目录中的 manifest 文本。
///
/// 落位 manifest 是上游 manifest 原样，`entry` 仍指向上游地址：装载不解析 `entry`
/// （wasm 固定为 `plugin.wasm`），故这里不加 entry 约束（见 ADR 0006）。
pub fn parse_placed(text: &str) -> Result<Manifest, String> {
    parse_metadata(text)
}

/// 解析并校验 metadata 与必填字段；不含按来源种类分列的 `entry` 约束。
fn parse_metadata(text: &str) -> Result<Manifest, String> {
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
        crate::plugin::testutil::manifest_json(id, "x", "x", "~/.x", entry)
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
    fn file_entry_rejects_non_https_schemes() {
        // 带 scheme 的 entry 只接受 https；http:// 等既不是相对路径也不会联网获取，
        // 必须在解析期拒绝，而不是当成相对路径去读一个不可能存在的文件。
        for entry in [
            "http://example.com/plugin.wasm",
            "file:///tmp/plugin.wasm",
            "ftp://example.com/plugin.wasm",
        ] {
            let err = parse_manifest(SourceKind::File, &manifest_text("pi", entry)).unwrap_err();
            assert!(
                err.contains("entry") && err.contains("https URL"),
                "entry {entry:?} 应被拒绝：{err}"
            );
        }
        assert!(
            parse_manifest(
                SourceKind::File,
                &manifest_text("pi", "https://example.com/plugin.wasm")
            )
            .is_ok(),
            "file 来源的 https entry 合法（回源到网络）"
        );
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

    #[test]
    fn source_kind_is_derived_from_the_source_string() {
        assert_eq!(
            SourceKind::from_source("builtin:pi"),
            Some(SourceKind::Builtin)
        );
        assert_eq!(
            SourceKind::from_source("https://example.com/manifest.json"),
            Some(SourceKind::Https)
        );
        assert_eq!(
            SourceKind::from_source("/tmp/plugin/manifest.json"),
            Some(SourceKind::File)
        );
        assert_eq!(
            SourceKind::from_source("plugins/pi/manifest.json"),
            None,
            "相对路径不是合法的来源身份（file 来源要求本机绝对路径）"
        );
        assert_eq!(SourceKind::from_source("file:///tmp/manifest.json"), None);
        assert_eq!(SourceKind::from_source("git://example.com/x.git"), None);
    }
}
