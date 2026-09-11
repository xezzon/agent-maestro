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
//! # 版本对齐约定
//!
//! SDK 版本与 WIT 包 `maestro:plugin` 版本按 semver 对齐：SDK 1.x ↔ WIT 1.x，
//! 合同 breaking 变更时同步升 major；minor/patch 在 major 内各自独立。
//! `maestro:plugin` v1 内只加不改（只新增字段、协议与函数）；宿主承诺 WIT 包
//! 版本升级时可同时注册新旧接口版本，给插件作者渐进迁移窗口。
//! WIT 合同的单一来源是仓库根的 `wit/maestro-plugin.wit`，本 crate 构建时
//! 内嵌该文件生成绑定（见 build.rs）。

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

/// `maestro:plugin` v1 合同的全部 WIT 类型与 `Guest` trait。
///
/// 类型与合同 v1 一一对应：加字段/加协议 = 合同升包版本 + 插件重编译。
pub use exports::maestro::plugin::plugin::{Guest, Model, Protocol, Provider};
