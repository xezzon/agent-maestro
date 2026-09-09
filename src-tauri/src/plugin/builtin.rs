/// 内置插件：随应用分发的插件，不经下载与安装；以字节内嵌于二进制，
/// 不物化到磁盘（见 ADR 0004）。可禁用，不可移除，不出现在添加流程。
///
/// manifest 来自仓库根的内置插件目录 `plugins/pi/`；wasm 产物不入库，
/// 由宿主 build.rs 在构建时自动编译（见 ADR 0005）。可禁用，不可移除，不出现在添加流程。
pub const BUILTIN_PI_SOURCE: &str = "builtin:pi";
pub const BUILTIN_PI_ID: &str = "pi";

pub const PI_MANIFEST_JSON: &str = include_str!("../../../plugins/pi/manifest.json");
pub const PI_WASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/pi-plugin.wasm"));

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::manifest::parse_manifest;

    #[test]
    fn embedded_pi_manifest_is_valid_and_matches_builtin_identity() {
        let manifest = parse_manifest(PI_MANIFEST_JSON).unwrap();

        assert_eq!(manifest.id, BUILTIN_PI_ID);
        assert_eq!(manifest.tool, "pi");
        assert_eq!(manifest.config_dir, "~/.pi");
        assert_eq!(manifest.entry, "plugin.wasm");
        assert!(!PI_WASM.is_empty(), "内置 pi wasm 产物非空");
    }
}
