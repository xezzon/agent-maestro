use std::{env, fs, path::PathBuf};

fn main() {
    let wit_path = locate_wit();
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR 未设置"));
    let bindings = format!(
        r#"wit_bindgen::generate!({{
    path: {wit_dir:?},
    world: "plugin-world",
    pub_export_macro: true,
    default_bindings_module: "::maestro_plugin_sdk",
}});"#,
        wit_dir = wit_path.parent().unwrap().display()
    );
    fs::write(out_dir.join("bindings.rs"), bindings).expect("无法写出 SDK 绑定生成文件");
    println!("cargo::rerun-if-changed={}", wit_path.display());
    println!("cargo::rerun-if-changed=build.rs");
}

/// 定位 WIT 合同：优先仓库根的 `wit/`（单一来源，始终生效——即使 crate 目录下
/// 残留了旧的发布副本也不会用它）；crates.io 包与 `cargo publish` 的校验构建
/// （包根之外没有仓库 wit/）回退到包内自带的 `wit/maestro-plugin.wit`，
/// 由 release 工作流在打包前拷入。
fn locate_wit() -> PathBuf {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR 未设置"));
    let repo = manifest_dir.join("../../wit/maestro-plugin.wit");
    let packaged = manifest_dir.join("wit/maestro-plugin.wit");
    if repo.is_file() {
        repo
    } else if packaged.is_file() {
        packaged
    } else {
        panic!(
            "找不到 WIT 合同：既无 {} 也无 {}（发布到 crates.io 前须由 release 流程把合同拷入包内）",
            repo.display(),
            packaged.display()
        );
    }
}
