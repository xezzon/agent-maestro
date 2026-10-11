use std::{
    env, fs,
    path::Path,
    process::{Command, Stdio},
};

fn main() {
    inject_app_version();
    build_builtin_plugin_wasm();
    tauri_build::build()
}

/// 应用版本号的唯一来源是 tauri.conf.json（Cargo.toml 已省略 version，见 ADR 0012）：
/// 读出来注入 rustc 环境变量，供 fetch.rs 的 User-Agent 使用——Cargo 自带的
/// CARGO_PKG_VERSION 随 `[package] version` 的省略而恒为 0.0.0。
///
/// 缺 version 字段即构建失败，而不是让 UA 静默退化成 0.0.0。
fn inject_app_version() {
    // tauri-build 对配置文件也会发这条指令，此处不依赖它的实现细节。
    println!("cargo::rerun-if-changed=tauri.conf.json");
    let raw = fs::read_to_string("tauri.conf.json").expect("tauri.conf.json 读取失败");
    let version = serde_json::from_str::<serde_json::Value>(&raw)
        .expect("tauri.conf.json 不是合法 JSON")
        .get("version")
        .and_then(|value| value.as_str())
        .expect("tauri.conf.json 缺少 version 字段")
        .to_owned();
    println!("cargo::rustc-env=AGENT_MAESTRO_APP_VERSION={version}");
}

/// 一个要编译的内置插件。
struct BuiltinPluginBuild {
    /// 插件 crate 目录，相对 src-tauri。
    crate_dir: &'static str,
    /// cargo 原生产物名（crate 名里的连字符在产物名中变为下划线）。
    artifact: &'static str,
    /// 复制到宿主 OUT_DIR 的文件名，供 builtin.rs 的 include_bytes! 引用。
    out_name: &'static str,
}

/// 要编译哪些内置插件：新增内置插件在此加一行，并在
/// `src-tauri/src/plugin/builtin.rs` 的 `BUILTINS` 表登记同一份产物。
const BUILTIN_PLUGINS: &[BuiltinPluginBuild] = &[
    BuiltinPluginBuild {
        crate_dir: "../plugins/pi",
        artifact: "maestro_plugin_pi.wasm",
        out_name: "pi-plugin.wasm",
    },
    BuiltinPluginBuild {
        crate_dir: "../plugins/claude-code",
        artifact: "maestro_plugin_claude_code.wasm",
        out_name: "claude-code-plugin.wasm",
    },
];

/// 内置插件共用的监视路径：插件都依赖 SDK crate（issue #41），SDK 源码或合同
/// 变更时重编全部内置 wasm。
const SDK_MARKERS: &[&str] = &[
    "../crates/maestro-plugin-sdk/wit",
    "../crates/maestro-plugin-sdk/src",
    "../crates/maestro-plugin-sdk/build.rs",
    "../crates/maestro-plugin-sdk/Cargo.toml",
];

/// 内置插件的 WASM 产物在构建宿主时自动编译，不入库（ADR 0005）：
/// 插件源码或 WIT 合同变更时重编，产物写到 OUT_DIR 供 builtin.rs 的 include_bytes! 引用。
fn build_builtin_plugin_wasm() {
    for plugin in BUILTIN_PLUGINS {
        build_plugin_artifact(plugin, SDK_MARKERS);
    }
}

/// 在宿主构建时把一个插件 crate 编译为 wasm32-wasip2 组件（`--release --locked`，
/// 隔离宿主构建的编译环境变量，避免外部 flag 泄入 wasm 编译），并把产物复制到
/// OUT_DIR 供 include_bytes! 引用。`markers` 为额外的 rerun-if-changed 路径
/// （SDK 源码与合同等）。
fn build_plugin_artifact(plugin: &BuiltinPluginBuild, markers: &[&str]) {
    let crate_dir = plugin.crate_dir;
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed={crate_dir}/src");
    println!("cargo::rerun-if-changed={crate_dir}/Cargo.toml");
    println!("cargo::rerun-if-changed={crate_dir}/Cargo.lock");
    // manifest.json 逐项列出而非盯整个插件目录：避免把 target（cargo 缓存）纳入监视。
    println!("cargo::rerun-if-changed={crate_dir}/manifest.json");
    for marker in markers {
        println!("cargo::rerun-if-changed={marker}");
    }

    let artifact = plugin.artifact;
    let built = format!("{crate_dir}/target/wasm32-wasip2/release/{artifact}");
    let out_path = Path::new(&env::var("OUT_DIR").expect("OUT_DIR 未设置")).join(plugin.out_name);

    let status = Command::new("cargo")
        .args([
            "build",
            "--release",
            "--locked",
            "--target",
            "wasm32-wasip2",
        ])
        .current_dir(crate_dir)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("CARGO_TARGET_DIR")
        // build script 的 stdout 会被 cargo 当作指令解析，进度与错误一律走 stderr（继承）。
        .stdout(Stdio::null())
        .status()
        .unwrap_or_else(|e| panic!("无法启动 cargo 编译插件 {crate_dir}：{e}"));
    if !status.success() {
        panic!(
            "编译插件 {crate_dir} 失败（exit {}），见上方 cargo 输出",
            status.code().unwrap_or(-1)
        );
    }
    fs::copy(&built, &out_path).unwrap_or_else(|e| panic!("插件 wasm 产物缺失（{built}）：{e}"));
}
