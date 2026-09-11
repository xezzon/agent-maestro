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

/// 定位 WIT 合同：随本 crate 分发的 `wit/maestro-plugin.wit`（单一来源，
/// 宿主 bindgen! 与插件绑定共用；随包发布即自包含）。
fn locate_wit() -> PathBuf {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR 未设置"));
    let wit_path = manifest_dir.join("wit/maestro-plugin.wit");
    if !wit_path.is_file() {
        panic!("找不到 WIT 合同：{}", wit_path.display());
    }
    wit_path
}
