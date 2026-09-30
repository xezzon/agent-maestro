//! `maestro-plugin-sdk`：Maestro 插件接口合同 `maestro:plugin` 的类型化 Rust 绑定。
//!
//! 插件作者只需依赖本 crate，实现 re-export 的 [`Guest`] trait，再以
//! [`export!`] 声明导出类型即可构建 wasm 组件，无需 vendor WIT 文件：
//!
//! ```ignore
//! use maestro_plugin_sdk::{export, Guest, Provider};
//!
//! struct MyPlugin;
//!
//! impl Guest for MyPlugin {
//!     fn write_providers(providers: Vec<Provider>) -> Result<Vec<String>, String> {
//!         // 把 Provider 投影进插件 manifest 声明的 config_dir（预开放为 "/"）。
//!         # Ok(Vec::new())
//!     }
//! }
//!
//! export!(MyPlugin);
//! ```
//!
//! # 依赖方式
//!
//! 本 crate 不发布到 crates.io，插件作者以 git 依赖引用本仓库中的 `crates/maestro-plugin-sdk`，
//! 并锁定 Maestro 的发布 tag——tag 与宿主版本一致，随 tag 固化插件与宿主的兼容组合：
//!
//! ```toml
//! [dependencies]
//! maestro-plugin-sdk = { git = "https://github.com/xezzon/agent-maestro", tag = "vX.Y.Z" }
//! ```
//!
//! 取含本 crate 的 release tag（首个此类 release 起可用）。
//!
//! # 版本对齐约定
//!
//! SDK 版本与 WIT 包 `maestro:plugin` 版本按 semver 对齐：SDK 1.x ↔ WIT 1.x，
//! 合同 breaking 变更时同步升 major；minor/patch 在 major 内各自独立。
//! `maestro:plugin` v1 内只加不改（只新增字段、协议与函数）；宿主承诺 WIT 包
//! 版本升级时可同时注册新旧接口版本，给插件作者渐进迁移窗口。
//! WIT 合同随本 crate 分发（`wit/maestro-plugin.wit`），构建时内嵌该文件
//! 生成绑定（见 build.rs）。

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

/// `maestro:plugin` v1 合同的全部 WIT 类型与 `Guest` trait。
///
/// 类型与合同 v1 一一对应：加字段/加协议 = 合同升包版本 + 插件重编译。
pub use exports::maestro::plugin::plugin::{Guest, Model, Protocol, Provider};

/// `maestro:plugin` v1.1.0 起新增的凭证桥接 import（见 issue #81）。
///
/// 宿主把目标工具的系统凭证库桥接为 [`keychain`] 的 `write` / `delete`：插件把
/// API Key 投影进目标工具自己的钥匙串条目，条目落在 manifest 声明的
/// `keychain_namespace`（目标工具自己的 service 名）下，`key` 是工具侧索引键。
/// **manifest 未声明 `keychain_namespace` 时调用即报错**——声明本身是权限闸门。
/// `delete` 对不存在的 key 视为成功（幂等）；同 key 重复写入为覆盖。
pub use maestro::plugin::keychain;
