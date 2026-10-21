/// 内置插件：随应用分发的插件，wasm 以字节内嵌于二进制（见 ADR 0005）；
/// 安装时与其余来源同一管线落位到磁盘并写条目（见 ADR 0004）。可禁用，不可移除。
///
/// manifest 来自仓库根的内置插件目录（`plugins/<id>/`）；wasm 产物不入库，
/// 由宿主 build.rs 在构建时自动编译（见 ADR 0005，构建清单见 `src-tauri/build.rs`
/// 的 `BUILTIN_PLUGINS`）。
/// 内置来源的 `source` 前缀：来源以内置前缀标识（见 docs/GLOSSARY.md「来源」）。
pub(crate) const SOURCE_PREFIX: &str = "builtin:";

pub(crate) const BUILTIN_PI_SOURCE: &str = "builtin:pi";
pub(crate) const BUILTIN_PI_ID: &str = "pi";
pub(crate) const BUILTIN_CLAUDE_CODE_SOURCE: &str = "builtin:claude-code";
pub(crate) const BUILTIN_CLAUDE_CODE_ID: &str = "claude-code";

pub(crate) const PI_MANIFEST_JSON: &str = include_str!("../../../plugins/pi/manifest.json");
pub(crate) const PI_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/pi-plugin.wasm"));
pub(crate) const CLAUDE_CODE_MANIFEST_JSON: &str =
    include_str!("../../../plugins/claude-code/manifest.json");
pub(crate) const CLAUDE_CODE_WASM: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/claude-code-plugin.wasm"));

/// 一个内置插件的来源、id 与内嵌产物。
pub(crate) struct BuiltinPlugin {
    /// 条目身份，`builtin:<id>`。
    pub(crate) source: &'static str,
    /// 插件 id（落位目录名、id 冲突检查与投影报告的键）。
    pub(crate) id: &'static str,
    pub(crate) manifest_json: &'static str,
    pub(crate) wasm: &'static [u8],
}

/// 内置插件清单：宿主按 `source` 精确查表分派，不再假设只有一个内置插件。
/// 与 `src-tauri/build.rs` 的 `BUILTIN_PLUGINS`（编译清单）配套——新增内置插件
/// 必须两处同步，因为编译清单在 build script 上下文、本表在宿主 crate 上下文。
pub(crate) const BUILTINS: &[BuiltinPlugin] = &[
    BuiltinPlugin {
        source: BUILTIN_PI_SOURCE,
        id: BUILTIN_PI_ID,
        manifest_json: PI_MANIFEST_JSON,
        wasm: PI_WASM,
    },
    BuiltinPlugin {
        source: BUILTIN_CLAUDE_CODE_SOURCE,
        id: BUILTIN_CLAUDE_CODE_ID,
        manifest_json: CLAUDE_CODE_MANIFEST_JSON,
        wasm: CLAUDE_CODE_WASM,
    },
];

/// 按来源精确查表；未知来源返回 `None`，由调用方报错而非回落（见 issue #56）。
pub(crate) fn find(source: &str) -> Option<&'static BuiltinPlugin> {
    BUILTINS.iter().find(|plugin| plugin.source == source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::manifest::{SourceKind, parse_manifest};

    #[test]
    fn embedded_pi_manifest_is_valid_and_matches_builtin_identity() {
        let root = tempfile::tempdir().unwrap();
        let dirs = crate::plugin::PlatformDirs::new(
            root.path().to_path_buf(),
            root.path().join(".config"),
            root.path().join(".config"),
            root.path().join(".local/share"),
            root.path().join(".local/share"),
        );
        let _g = crate::plugin::PlatformDirs::test_guard(dirs);
        let manifest = parse_manifest(SourceKind::Builtin, PI_MANIFEST_JSON).unwrap();

        assert_eq!(manifest.id, BUILTIN_PI_ID);
        assert_eq!(
            manifest.config_dir,
            root.path().canonicalize().unwrap().join(".pi")
        );
        assert_eq!(
            manifest.entry, "target/wasm32-wasip2/release/maestro_plugin_pi.wasm",
            "entry 指向 cargo 原生产物路径：同一份 manifest 兼作 file 来源本地调试的入口"
        );
        assert!(!PI_WASM.is_empty(), "内置 pi wasm 产物非空");
    }
}
