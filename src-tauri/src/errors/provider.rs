//! Provider 仓储的错误：重复/缺失与插值校验失败。

use std::fmt::{self, Display};

use super::{config::ConfigError, impl_storage_error, interpolation::ProviderInterpolationError};

/// Provider 仓储的错误：重复/缺失与插值校验失败。
#[derive(Debug, Clone)]
pub(crate) enum ProviderError {
    /// 已存在同名 Provider。
    DuplicateSlug { slug: String },
    /// Provider 不存在（update/delete 不做 upsert，绝不静默覆盖）。
    MissingSlug { slug: String },
    /// Provider 对（变更后的）全局变量表插值失败：变量未定义或占位符语法
    /// 错误。与投影时的插值失败同因同报，只是提前到落盘前拦截
    /// （provider↔variables 一致性，ADR 0015）。
    Interpolation(ProviderInterpolationError),
    /// 配置的读取或落盘失败。
    Storage(ConfigError),
}

impl ProviderError {
    pub(crate) fn duplicate_slug(slug: &str) -> Self {
        Self::DuplicateSlug {
            slug: slug.to_owned(),
        }
    }

    pub(crate) fn missing_slug(slug: &str) -> Self {
        Self::MissingSlug {
            slug: slug.to_owned(),
        }
    }
}

impl_storage_error!(ProviderError);

impl Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderError::DuplicateSlug { slug } => write!(f, "已存在同名 Provider：{slug}"),
            ProviderError::MissingSlug { slug } => write!(f, "Provider 不存在：{slug}"),
            ProviderError::Interpolation(error) => write!(f, "{error}"),
            ProviderError::Storage(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ProviderError {}
