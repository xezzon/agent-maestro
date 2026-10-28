//! 密钥链基座（ADR 0002）：凭证类型、密钥链端口与平台实现。
//!
//! 模块回答三个问题：
//!
//! - **密钥字段长什么样**：[`SecretText`] 表达「未设置 / 密钥链引用 / 明文」
//!   三态（字段允不允许明文由字段自己决定），[`SecretRef`] 是引用的规范形态
//!   （`secret://` URI）；
//! - **真值怎么读写**：[`SecretStore`] 端口——读、写、删三个动作，条目以
//!   [`SecretRef`] 定位；生产实现是 [`KeyringSecrets`]（系统密钥链），测试
//!   替身是 `MemorySecrets`（内存）；
//! - **不支持的平台怎么办**：不支持的编译目标**编译期即失败**（见本模块的
//!   `compile_error!`），绝不静默回退到内存实现——内存实现只在测试里存在。
//!
//! 模块内不出现真值：[`SecretText`] 的 `Debug` 只渲染「配没配」，
//! [`SecretError`](crate::errors::SecretError) 的原因不携带密钥值。
//!
//! 结构：
//! - [`reference`]：密钥引用的规范形态与严格解析；
//! - [`text`]：凭证字段的三态；
//! - [`store`]：密钥链端口；
//! - [`keyring`]：系统密钥链（唯一生产实现）；
//! - `memory`：内存密钥链（仅测试）。

// 基座票（#144）：类型与端口已就位，接线在 #145（Store 注入
// `Arc<dyn SecretStore>`）。在那之前没有人调用本模块的公开面，rustc 会把
// 未被使用的类型与再导出判为 dead code / unused import——此处显式放行，
// #145 接线后删掉这一行。
#![allow(dead_code, unused_imports)]

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod keyring;
#[cfg(test)]
mod memory;
mod reference;
mod store;
mod text;

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) use keyring::KeyringSecrets;
#[cfg(test)]
pub(crate) use memory::MemorySecrets;
pub(crate) use reference::SecretRef;
pub(crate) use store::{SecretOp, SecretStore};
pub(crate) use text::SecretText;

// 本模块支持的编译目标：系统密钥链只在这些平台上有实现。
//
// 不在名单里的目标**编译期即失败**（见下面的 `compile_error!`）：密钥链
// 不可用时要如实报错（ADR 0002），绝不静默回退到内存实现——那等于把真值
// 留在进程内存里，用户以为存进了密钥链，实际没有。
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
compile_error!(
    "系统密钥链只在 macOS / Windows / Linux 上有实现；\
     当前编译目标不受支持，绝不回退到内存实现（ADR 0002）"
);
