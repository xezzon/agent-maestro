/// 内置插件：随应用分发的插件，wasm 以字节内嵌于二进制（见 ADR 0005）；
/// 安装时与其余来源同一管线落位到磁盘并写条目（见 ADR 0004）。可禁用，不可移除。
///
/// manifest 来自仓库根的内置插件目录（`plugins/<id>/`）；wasm 产物不入库，
/// 由宿主 build.rs 在构建时自动编译（见 ADR 0005，构建清单见 `src-tauri/build.rs`
/// 的 `BUILTIN_PLUGINS`）。
/// 内置来源的前缀：来源以内置前缀标识（见 docs/GLOSSARY.md「来源」）。
pub(crate) const SOURCE_PREFIX: &str = "builtin:";

/// 内置插件 id。
///
/// 来源（条目身份）由 id 派生（`builtin:<id>`），不另写一份——同一信息在
/// 常量与清单表里各写一次，改一处漏一处就是「找不到插件」。
pub(crate) const BUILTIN_PI_ID: &str = "pi";
pub(crate) const BUILTIN_CLAUDE_CODE_ID: &str = "claude-code";

/// 内置来源的条目身份 `builtin:<id>`。
pub(crate) fn source_of(id: &str) -> String {
    format!("{SOURCE_PREFIX}{id}")
}

pub(crate) const PI_MANIFEST_JSON: &str = include_str!("../../../plugins/pi/manifest.json");
pub(crate) const PI_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/pi-plugin.wasm"));
pub(crate) const CLAUDE_CODE_MANIFEST_JSON: &str =
    include_str!("../../../plugins/claude-code/manifest.json");
pub(crate) const CLAUDE_CODE_WASM: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/claude-code-plugin.wasm"));

/// 一个内置插件的 id 与内嵌产物。
pub(crate) struct BuiltinPlugin {
    /// 插件 id（落位目录名、id 冲突检查与投影报告的键）。
    pub(crate) id: &'static str,
    pub(crate) manifest_json: &'static str,
    pub(crate) wasm: &'static [u8],
}

impl BuiltinPlugin {
    /// 条目身份 `builtin:<id>`：由 id 派生，不另存一份来源字符串。
    pub(crate) fn source(&self) -> String {
        source_of(self.id)
    }
}

/// 内置插件清单：宿主按 id 精确查表分派，不再假设只有一个内置插件。
/// 与 `src-tauri/build.rs` 的 `BUILTIN_PLUGINS`（编译清单）配套——新增内置插件
/// 必须两处同步，因为编译清单在 build script 上下文、本表在宿主 crate 上下文。
pub(crate) const BUILTINS: &[BuiltinPlugin] = &[
    BuiltinPlugin {
        id: BUILTIN_PI_ID,
        manifest_json: PI_MANIFEST_JSON,
        wasm: PI_WASM,
    },
    BuiltinPlugin {
        id: BUILTIN_CLAUDE_CODE_ID,
        manifest_json: CLAUDE_CODE_MANIFEST_JSON,
        wasm: CLAUDE_CODE_WASM,
    },
];

/// 按插件 id 精确查表；未知 id 返回 `None`，由调用方报错而非回落（见 issue #56）。
pub(crate) fn find_by_id(id: &str) -> Option<&'static BuiltinPlugin> {
    BUILTINS.iter().find(|plugin| plugin.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::{manifest::parse_manifest, source::PluginSource};

    #[test]
    fn embedded_pi_manifest_is_valid_and_matches_builtin_identity() {
        let root = tempfile::tempdir().unwrap();
        let _g = crate::paths::Paths::test_guard(crate::paths::Paths::from_home(root.path()));
        let manifest = parse_manifest(
            &PluginSource::Builtin(BUILTIN_PI_ID.to_owned()),
            PI_MANIFEST_JSON,
        )
        .unwrap();

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
