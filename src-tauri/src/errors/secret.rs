//! 密钥链的错误：区分**哪一步失败**与**为什么失败**。
//!
//! 密钥链的失败有两类，处置方式完全不同：
//!
//! - **条目不存在**：不是失败——读时读作「没有值」，删时视作成功（幂等）。
//!   它在 [`SecretCause::NoEntry`] 里单独成因，就是为了让它与「密钥链
//!   不可用」泾渭分明：把二者混同，要么把「后端缺席」当成「没有值」（真值
//!   被静默吞掉），要么把「条目缺失」当成故障（清除操作永远失败）。
//! - **密钥链不可用**：后端缺席、访问被拒、条目库锁着——任何操作都可能因它
//!   失败，调用方必须如实报错，绝不回退为明文（ADR 0002）。
//!
//! 错误**不携带密钥值**：原因只描述平台侧的失败，不含真值，可以直接写进日志。

use std::fmt::{self, Display};

/// 密钥链操作的失败：读、写、删各一。
///
/// 成因另在 [`SecretCause`] 里：同一成因可出现在任一操作上，故两个维度分开
/// 表达——「哪一步失败」与「为什么失败」都永远说得清。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SecretError {
    /// 读取失败：条目在，取不出值。
    Read { cause: SecretCause },
    /// 写入失败：值写不进密钥链。
    Write { cause: SecretCause },
    /// 删除失败：条目删不掉。
    Delete { cause: SecretCause },
}

/// 密钥链失败的成因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SecretCause {
    /// 条目不存在：读时读作「没有值」、删时视作成功——两者都不是失败。
    ///
    /// 端口层（[`crate::secret::SecretStore`]）把这个成因归一掉，故它不会
    /// 作为失败返回给调用方；保留这个成因是为了让「不存在」与「不可用」在
    /// 分类上分得开。
    NoEntry,
    /// 密钥链不可用：后端缺席、连不上、访问被拒或条目库锁着。
    Unavailable { reason: String },
    /// 平台拒绝了这次请求：值编码不对、属性超长、同名条目多条等。
    Rejected { reason: String },
}

impl Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecretError::Read { cause } => write!(f, "读取系统密钥链失败：{cause}"),
            SecretError::Write { cause } => write!(f, "写入系统密钥链失败：{cause}"),
            SecretError::Delete { cause } => write!(f, "删除系统密钥链条目失败：{cause}"),
        }
    }
}

impl Display for SecretCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecretCause::NoEntry => write!(f, "条目不存在"),
            SecretCause::Unavailable { reason } => write!(f, "密钥链不可用：{reason}"),
            SecretCause::Rejected { reason } => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for SecretError {}

/// 密钥引用解析失败：引用的形态不合规。
///
/// **不携带被拒绝的原文**：以 `secret://` 开头却不合规的字符串，可能是一条
/// 恰好撞上保留前缀的旧明文密钥；把它写进错误消息等于把密钥写进日志
/// （ADR 0002）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SecretRefError {
    /// 不是 `secret://` 开头的引用。
    NotAReference,
    /// 命名空间不对：不是本应用的固定命名空间。
    WrongService,
    /// account 形态不对：不是 `provider/<slug>/api_key`。
    WrongAccount,
    /// slug 不合法：不符合 Provider 的 slug 规则。
    InvalidSlug,
}

impl Display for SecretRefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecretRefError::NotAReference => write!(f, "不是密钥引用：应以 secret:// 开头"),
            SecretRefError::WrongService => {
                write!(f, "密钥引用的命名空间不是 io.github.xezzon.agent-maestro")
            }
            SecretRefError::WrongAccount => {
                write!(f, "密钥引用不是 provider/<slug>/api_key 形态")
            }
            SecretRefError::InvalidSlug => write!(f, "密钥引用里的 slug 不合法"),
        }
    }
}

impl std::error::Error for SecretRefError {}
