//! Provider 仓储的错误：重复/缺失、插值校验失败与凭证操作失败。

use std::fmt::{self, Display};

use super::{
    config::ConfigError, impl_storage_error, interpolation::ProviderInterpolationError,
    secret::SecretError,
};

/// Provider 仓储的错误：重复/缺失、插值校验失败与凭证操作失败。
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
    /// 载荷里的 `api_key` 是密钥引用：引用只由 Maestro 自己写入。
    ///
    /// 前端回传的引用一律不接受（规范引用在更新时按「不变」处理，由命令层
    /// 先行归一；走到这里的就是别的 Provider 的引用或不成形的引用）。
    UnexpectedSecretReference { slug: String },
    /// slug 不合法：无法为它构造规范密钥引用。
    ///
    /// 引用与 slug 绑定（GLOSSARY「密钥引用」），构造不出引用就没有地方放
    /// 真值——宁可报错，也不把真值退回配置文件（ADR 0002）。
    InvalidSlug { slug: String },
    /// 密钥链操作失败：真值写不进 / 读不出 / 删不掉。
    ///
    /// 密钥链不可用时如实报错，绝不回退为明文（ADR 0002）。
    Secret { slug: String, error: SecretError },
    /// 配置落盘失败，且回滚密钥链也失败：两次失败一并点名。
    ///
    /// 配置没改动，但密钥链可能与配置不一致——用户需要知道两处都要看一眼。
    SecretRollback {
        slug: String,
        /// 落盘失败的原因（装箱：错误类型本身已够大，不再让每个错误都背着它）。
        persist: Box<ConfigError>,
        /// 回滚密钥链失败的原因。
        rollback: SecretError,
    },
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

    pub(crate) fn unexpected_secret_reference(slug: &str) -> Self {
        Self::UnexpectedSecretReference {
            slug: slug.to_owned(),
        }
    }

    pub(crate) fn invalid_slug(slug: &str) -> Self {
        Self::InvalidSlug {
            slug: slug.to_owned(),
        }
    }

    pub(crate) fn secret(slug: &str, error: SecretError) -> Self {
        Self::Secret {
            slug: slug.to_owned(),
            error,
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
            ProviderError::UnexpectedSecretReference { slug } => write!(
                f,
                "Provider「{slug}」的 api_key 不接受密钥引用：引用只由 Maestro 自己写入，请直接填写真实凭证"
            ),
            ProviderError::InvalidSlug { slug } => write!(
                f,
                "Provider 的 slug 不合法：{slug}（密钥引用要求 slug 以小写字母开头，只含小写字母、数字、连字符与下划线）"
            ),
            ProviderError::Secret { slug, error } => {
                write!(f, "Provider「{slug}」的凭证操作失败：{error}")
            }
            ProviderError::SecretRollback {
                slug,
                persist,
                rollback,
            } => write!(
                f,
                "Provider「{slug}」保存失败：配置落盘失败（{persist}）；回滚密钥链也失败（{rollback}）。\n配置未改动，但密钥链可能与配置不一致，请检查系统密钥链。"
            ),
            ProviderError::Storage(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ProviderError {}
