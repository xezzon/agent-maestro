//! 错误类型：按领域拆分，每个类型自己排版面向用户的文案（[`std::fmt::Display`]）。
//!
//! 错误类型大多住在 crate 根而非某一层：配置存储与插件服务两侧都消费其中一部分
//! （插件服务经端口收存储错误，命令层把两侧的错误都转成字符串交给 IPC），
//! 放在中立层才不会让两侧互相依赖成环。路径解析失败（`paths::PathError`）例外：
//! 它只有一个消费者，随 [`crate::paths`] 住，`paths` 因此不反向依赖本模块。
//!
//! 存储错误按仓储职责拆开：[`ConfigError`]（配置文档读写）、[`ProviderError`]、
//! [`PluginEntryError`] 与 [`VariableError`]。各自把持久化失败包在 `Storage`
//! 变体里——调用方只关心「这次操作失败了」，无需在签名里同时列出领域错误与
//! 存储错误。插件服务另有 [`PluginError`]，它把来自端口的两类存储错误与自己的
//! 领域错误拼在一起。
//!
//! 每个领域住在自己的子模块里（[`config`] / [`provider`] / [`entry`] /
//! [`variable`] / [`interpolation`] / [`fetch`] / [`logging`] / [`plugin`]），
//! 本模块只做两件事：把类型 re-export 出去（`crate::errors::X` 的路径不变，
//! 调用方零改动），以及收纳跨领域的契约（见 [`impl_storage_error`]）。

pub(crate) use config::{ConfigError, LockPoisoned};
pub(crate) use entry::PluginEntryError;
pub(crate) use fetch::FetchError;
pub(crate) use interpolation::{FieldError, InterpolationError, ProviderInterpolationError};
pub(crate) use logging::LoggingError;
pub(crate) use plugin::PluginError;
pub(crate) use provider::ProviderError;
pub(crate) use variable::VariableError;

mod config;
mod entry;
mod fetch;
mod interpolation;
mod logging;
mod plugin;
mod provider;
mod variable;

/// 存储错误域的共有形状：把配置错误与锁毒化折进本域的 `Storage` 变体。
///
/// 三个存储错误域（Provider / 插件条目 / 变量表）共用同一形状。宏一次生成两个
/// `From` 实现——手抄三份的样板迟早各自漂移，而这两个 `From` 正是「锁毒化折成
/// 一句『配置存储不可用』」这条契约的落点。
macro_rules! impl_storage_error {
    ($error:ident) => {
        impl From<crate::errors::ConfigError> for $error {
            fn from(error: crate::errors::ConfigError) -> Self {
                Self::Storage(error)
            }
        }

        impl From<crate::errors::LockPoisoned> for $error {
            fn from(_: crate::errors::LockPoisoned) -> Self {
                Self::Storage(crate::errors::ConfigError::Unavailable)
            }
        }
    };
}

pub(crate) use impl_storage_error;

#[cfg(test)]
mod tests {
    use super::*;

    /// 落盘前拦截的插值失败文案逐字断言：slug、字段名、原因三者都在句子里，
    /// 且与投影时拦截同一句式（ADR 0015「报错与投影时同因同报」；句式由
    /// [`ProviderInterpolationError`] 独家产出）。
    #[test]
    fn interpolation_error_message_is_verbatim() {
        let error = ProviderInterpolationError {
            slug: "gateway".to_owned(),
            error: FieldError {
                field: "base_url.openai-completions".to_owned(),
                reason: "No such variable: $HOST".to_owned(),
            },
        };

        assert_eq!(
            ProviderError::Interpolation(error.clone()).to_string(),
            "Provider「gateway」的 base_url.openai-completions 插值失败：No such variable: $HOST"
        );
        assert_eq!(
            VariableError::Interpolation(error).to_string(),
            "Provider「gateway」的 base_url.openai-completions 插值失败：No such variable: $HOST"
        );
    }

    #[test]
    fn slug_and_source_error_messages_are_verbatim() {
        assert_eq!(
            ProviderError::DuplicateSlug {
                slug: "foo".to_owned()
            }
            .to_string(),
            "已存在同名 Provider：foo"
        );
        assert_eq!(
            ProviderError::MissingSlug {
                slug: "ghost".to_owned()
            }
            .to_string(),
            "Provider 不存在：ghost"
        );
        assert_eq!(
            PluginEntryError::MissingSource {
                source: "builtin:pi".to_owned()
            }
            .to_string(),
            "插件条目不存在：builtin:pi"
        );
        assert_eq!(
            PluginEntryError::DuplicateSource {
                source: "https://example.com/manifest.json".to_owned()
            }
            .to_string(),
            "已存在同一来源的插件条目：https://example.com/manifest.json"
        );
        assert_eq!(
            PluginEntryError::DuplicateId {
                id: "pi".to_owned(),
                source: "builtin:pi".to_owned()
            }
            .to_string(),
            "插件 id「pi」已被来源 builtin:pi 占用"
        );
    }

    /// 锁毒化折进各错误域后仍是同一句文案：毒化的是锁，不是配置。
    #[test]
    fn lock_poisoned_folds_into_the_same_reason_for_every_error_domain() {
        assert_eq!(
            ConfigError::from(LockPoisoned).to_string(),
            "配置存储不可用"
        );
        assert_eq!(
            ProviderError::from(LockPoisoned).to_string(),
            "配置存储不可用"
        );
        assert_eq!(
            PluginEntryError::from(LockPoisoned).to_string(),
            "配置存储不可用"
        );
        assert_eq!(
            VariableError::from(LockPoisoned).to_string(),
            "配置存储不可用"
        );
    }
}
