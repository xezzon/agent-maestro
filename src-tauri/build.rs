use std::{
    env, fs,
    path::Path,
    process::{Command, Stdio},
};

fn main() {
    build_builtin_plugin_wasm();
    tauri_build::build()
}

/// 内置 pi 插件的 WASM 产物在构建宿主时自动编译，不入库（ADR 0005）：
/// 插件源码或 WIT 合同变更时重编，产物写到 OUT_DIR 供 builtin.rs 的 include_bytes! 引用。
fn build_builtin_plugin_wasm() {
    // 逐项列出而非盯整个插件目录：避免把 plugins/pi/target（cargo 缓存）纳入监视。
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=../plugins/pi/src");
    println!("cargo::rerun-if-changed=../plugins/pi/Cargo.toml");
    println!("cargo::rerun-if-changed=../plugins/pi/Cargo.lock");
    println!("cargo::rerun-if-changed=../plugins/pi/manifest.json");
    println!("cargo::rerun-if-changed=../crates/maestro-plugin-sdk/wit");
    // pi 插件依赖 SDK crate（issue #41）：SDK 源码或合同变更时重编内置 wasm。
    println!("cargo::rerun-if-changed=../crates/maestro-plugin-sdk/src");
    println!("cargo::rerun-if-changed=../crates/maestro-plugin-sdk/build.rs");
    println!("cargo::rerun-if-changed=../crates/maestro-plugin-sdk/Cargo.toml");

    let out_path = Path::new(&env::var("OUT_DIR").expect("OUT_DIR 未设置")).join("pi-plugin.wasm");
    let status = Command::new("cargo")
        .args([
            "build",
            "--release",
            "--locked",
            "--target",
            "wasm32-wasip2",
        ])
        .current_dir("../plugins/pi")
        // 隔离宿主构建的编译环境变量，避免外部 flag 泄入 wasm 编译。
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("CARGO_TARGET_DIR")
        // build script 的 stdout 会被 cargo 当作指令解析，进度与错误一律走 stderr（继承）。
        .stdout(Stdio::null())
        .status()
        .unwrap_or_else(|e| panic!("无法启动 cargo 编译内置插件：{e}"));
    if !status.success() {
        panic!(
            "编译内置插件失败（exit {}），见上方 cargo 输出",
            status.code().unwrap_or(-1)
        );
    }
    fs::copy(
        "../plugins/pi/target/wasm32-wasip2/release/maestro_plugin_pi.wasm",
        &out_path,
    )
    .expect("内置插件 wasm 产物缺失：cargo build 成功但未生成期望产物");
}
