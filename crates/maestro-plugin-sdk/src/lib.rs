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
//! # 插件配置
//!
//! 宿主在实例化之前把插件配置中 `form` 段的原始 JSON 文本就位
//! （`maestro:plugin` v2.1.0 起，见 ADR 0018），组件在 init 与
//! write-providers 之前均可经 [`get_config`] 读取：
//!
//! ```ignore
//! use maestro_plugin_sdk::get_config;
//!
//! if let Some(form) = get_config() {
//!     // form 是插件配置 form 段的原始 JSON 文本，由插件自行解析。
//! }
//! ```
//!
//! 配置缺省或无该段时返回 [`Option::None`]。宿主只透传原始文本，不解析
//! 也不审查。
//!
//! # Provider 自定义 header 与模型上限、能力
//!
//! `maestro:plugin` v2.2.0 起，`Provider` 多带一份跨协议、跨模型共享的自定义
//! HTTP header（`custom-header`，宿主按名排序），`Model` 多带声明式的 token
//! 上限（`limit`，三个子项各自可缺省，未设置即 `none`）与能力集合
//! （`capabilities`，宿主按枚举声明序排序并去重）。这些字段只作声明：宿主不
//! 据此推断、不裁剪请求、不做跨字段一致性校验，怎么投影进工具配置由插件裁定
//! （见 ADR 0019）。header 的值已完成宿主插值且**可能含凭证**，不得写进日志。
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
//! SDK 版本与 WIT 包 `maestro:plugin` 版本按 semver 对齐：SDK 2.x ↔ WIT 2.x，
//! 合同 breaking 变更时同步升 major；minor/patch 在 major 内各自独立。
//! `maestro:plugin` 在 major 内只加不改（只新增字段、协议与函数）；WIT 包版本
//! 升级（如 1.1.0 → 2.0.0）后宿主只注册当前版本，插件须以新版 SDK 重编译
//! （宿主恒定提供的 import 不改变这一要求）。
//! WIT 合同随本 crate 分发（`wit/maestro-plugin.wit`），构建时内嵌该文件
//! 生成绑定（见 build.rs）。

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

/// `maestro:plugin` v2 合同的全部 WIT 类型与 `Guest` trait。
///
/// 类型与合同 v2 一一对应：加字段/加协议 = 合同升包版本 + 插件重编译。
pub use exports::maestro::plugin::plugin::{
    CustomHeader, Endpoint, Guest, Model, ModelCapability, ModelLimit, Protocol, Provider,
};

/// 过程性日志 import 绑定（`maestro:plugin` v1.1.0 起，见 [`log`]）：
/// [`Level`] 枚举与 [`log`] 函数 re-export 到 crate 根，方便插件一行引入。
pub use maestro::plugin::logger::{Level, log};

/// 插件配置 import 绑定（`maestro:plugin` v2.1.0 起，见 [`get_config`]）：
/// `get_config` 函数 re-export 到 crate 根，方便插件一行引入。
pub use maestro::plugin::config::get_config;
