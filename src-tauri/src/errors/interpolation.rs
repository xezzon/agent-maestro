//! 插值的错误：底层库的报错原文与字段级失败。
//!
//! 这一域是「同一句式只有一处实现」的锚点——落盘前拦截
//! （[`crate::errors::ProviderError::Interpolation`] /
//! [`crate::errors::VariableError::Interpolation`] 包裹本类型）与投影时拦截
//! （调用方直接格式化本类型）据此同因同报、逐字一致。

use std::fmt::{self, Display};

/// 占位符插值失败：底层库的报错原文（只含变量名与位置，不含值与密钥）。
#[derive(Debug, Clone)]
pub(crate) struct InterpolationError {
    detail: String,
}

impl InterpolationError {
    pub(crate) fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl Display for InterpolationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.detail)
    }
}

impl std::error::Error for InterpolationError {}

/// 插值失败：字段名 + 原因。
///
/// 字段名指明插的是哪个字段（`base_url.openai-completions`、`models[1].display_name`），
/// 原因取自库（只含变量名与位置，不含值与密钥）。
///
/// 用具名字段而非匿名元组：元组的两个位置写反了编译器不报错，解构点读到的
/// 字段名与原因会悄悄对调。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FieldError {
    /// 出错的字段名。
    pub(crate) field: String,
    /// 原因（库报错）。
    pub(crate) reason: String,
}

/// 跨 Provider 插值失败：出错的条目（slug）与其字段级失败。
///
/// 面向用户的句式由本类型独家产出（见 [`Display`]）：slug + 字段名 + 原因入句。
/// 落盘前拦截（[`crate::errors::ProviderError::Interpolation`] /
/// [`crate::errors::VariableError::Interpolation`] 包裹本类型）与投影时拦截
/// （调用方直接格式化本类型）据此同因同报、逐字一致——同一句式只有这一处实现。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderInterpolationError {
    /// 出错的 Provider（slug）。
    pub(crate) slug: String,
    /// 字段级失败：字段名 + 库报错。
    pub(crate) error: FieldError,
}

impl Display for ProviderInterpolationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Provider「{}」的 {} 插值失败：{}",
            self.slug, self.error.field, self.error.reason
        )
    }
}

impl std::error::Error for ProviderInterpolationError {}
