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
//! # 过程性日志
//!
//! 投影期间需要向用户外显的非致命事件（逐条容错跳过、降级提示等）经
//! [`log`] 上报（`maestro:plugin` v1.1.0 起，见 issue #82）：
//!
//! ```ignore
//! use maestro_plugin_sdk::{log, Level};
//!
//! log(Level::Warning, "模型 x 已弃用，仍继续投影");
//! ```
//!
//! 宿主把每条消息按 level 写入应用日志（error→ERROR、warning→WARN、
//! info→INFO、debug→DEBUG；默认级别 INFO，debug 需 `MAESTRO_LOG_LEVEL=debug`
//! 才落盘），消息不进投影结果。message **不得携带 API Key 等凭证**——宿主
//! 不做内容审查，秘密一旦上报即会进入日志文件（见 ADR 0008）。
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
//! `maestro:plugin` v1 内只加不改（只新增字段、协议与函数）；WIT 包版本升级
//! （如 1.0.0 → 1.1.0）后宿主只注册当前版本，插件须以新版 SDK 重编译
//! （宿主恒定提供的 import 不改变这一要求）。
//! WIT 合同随本 crate 分发（`wit/maestro-plugin.wit`），构建时内嵌该文件
//! 生成绑定（见 build.rs）。

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

/// `maestro:plugin` v1 合同的全部 WIT 类型与 `Guest` trait。
///
/// 类型与合同 v1 一一对应：加字段/加协议 = 合同升包版本 + 插件重编译。
pub use exports::maestro::plugin::plugin::{Guest, Model, Protocol, Provider};

/// 过程性日志 import 绑定（`maestro:plugin` v1.1.0，见 [`log`]）：
/// [`Level`] 枚举与 [`log`] 函数 re-export 到 crate 根，方便插件一行引入。
pub use maestro::plugin::logger::{Level, log};
