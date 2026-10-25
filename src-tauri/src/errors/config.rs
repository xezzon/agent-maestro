//! 配置存储的错误：配置文件本身的读取、解析、序列化与落盘。
//!
//! 锁毒化（[`LockPoisoned`]）也住在这一域——被毒化的是 `AppStore` 的锁，
//! 不是配置本身。

use std::{
    fmt::{self, Display},
    path::{Path, PathBuf},
};

/// 配置存储的锁被毒化（其它命令持锁期间 panic）。
///
/// 这不是任何领域错误的一种：毒化的是 `AppStore` 的锁而非配置本身。锁边界
/// 经 [`From`] 把它折进调用方的错误类型——每个可经由锁触发的错误类型都有一段
/// `From<LockPoisoned>`，统一归到「配置存储不可用」这一句。
pub(crate) struct LockPoisoned;

/// 配置存储不可用时对用户可见的原因。
///
/// 只由锁毒化产出：保护态（配置文件损坏）带的是自己的原因（损坏 / 版本不受支持），
/// 不走这一句。
const STORE_LOCK_POISONED: &str = "配置存储不可用";

/// 配置文件本身的错误：读取、解析、序列化与落盘。
#[derive(Debug, Clone)]
pub(crate) enum ConfigError {
    /// 配置文件存在但无法解析。
    Corrupt { path: PathBuf, detail: String },
    /// 配置文件版本不被当前应用支持。
    UnsupportedVersion {
        path: PathBuf,
        found: u32,
        /// 当前应用支持的版本。由存储层在比较时填入——错误类型不反向依赖
        /// [`crate::config`]（#141 无环约束）。
        supported: u32,
    },
    /// 读写配置文件时发生 IO 错误。
    Io { path: PathBuf, detail: String },
    /// 配置读得出来，却写不出合法的 JSON（序列化失败）。
    ///
    /// 与 IO 失败是两件事：文件没坏、也不是写不动，而是内容写不成 JSON——
    /// 混进 `Io` 会让报错指向错误的修复方向（去查磁盘权限，而不是查内容）。
    Serialize { detail: String },
    /// 配置存储的锁不可用（其它命令持锁期间 panic 毒化了锁）。
    Unavailable,
}

impl ConfigError {
    pub(crate) fn corrupt(path: &Path, detail: impl Into<String>) -> Self {
        Self::Corrupt {
            path: path.to_path_buf(),
            detail: detail.into(),
        }
    }

    pub(crate) fn io(path: &Path, detail: impl Into<String>) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            detail: detail.into(),
        }
    }

    pub(crate) fn serialize(detail: impl Into<String>) -> Self {
        Self::Serialize {
            detail: detail.into(),
        }
    }
}

impl From<LockPoisoned> for ConfigError {
    fn from(_: LockPoisoned) -> Self {
        Self::Unavailable
    }
}

impl Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Corrupt { path, detail } => write!(
                f,
                "配置文件已损坏：{}\n原因：{detail}\n请修复或删除该文件后重启应用；在此之前 Maestro 拒绝任何写入，绝不会静默重建。",
                path.display()
            ),
            ConfigError::UnsupportedVersion {
                path,
                found,
                supported,
            } => write!(
                f,
                "配置文件版本不受支持：{}\n文件中的 version 为 {found}，当前应用仅支持 {supported}。\n请修复该文件后重启应用；在此之前 Maestro 拒绝任何写入，绝不会静默重建。",
                path.display()
            ),
            ConfigError::Io { path, detail } => {
                write!(f, "读写配置文件失败：{}\n原因：{detail}", path.display())
            }
            ConfigError::Serialize { detail } => {
                write!(f, "序列化配置文件失败\n原因：{detail}")
            }
            ConfigError::Unavailable => write!(f, "{STORE_LOCK_POISONED}"),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// 保护态文案逐字钉住：路径、原因与修复指引三段排版都在。
    #[test]
    fn corrupt_error_message_is_verbatim() {
        let err = ConfigError::Corrupt {
            path: PathBuf::from("/home/u/.maestro/config.json"),
            detail: "expected value at line 1 column 1".to_owned(),
        };

        assert_eq!(
            err.to_string(),
            "配置文件已损坏：/home/u/.maestro/config.json\n原因：expected value at line 1 column 1\n请修复或删除该文件后重启应用；在此之前 Maestro 拒绝任何写入，绝不会静默重建。"
        );
    }

    #[test]
    fn unsupported_version_error_message_is_verbatim() {
        let err = ConfigError::UnsupportedVersion {
            path: PathBuf::from("/home/u/.maestro/config.json"),
            found: 99,
            supported: 1,
        };

        assert_eq!(
            err.to_string(),
            "配置文件版本不受支持：/home/u/.maestro/config.json\n文件中的 version 为 99，当前应用仅支持 1。\n请修复该文件后重启应用；在此之前 Maestro 拒绝任何写入，绝不会静默重建。"
        );
    }

    #[test]
    fn io_error_message_is_verbatim() {
        let err = ConfigError::Io {
            path: PathBuf::from("/home/u/.maestro/config.json"),
            detail: "Permission denied (os error 13)".to_owned(),
        };

        assert_eq!(
            err.to_string(),
            "读写配置文件失败：/home/u/.maestro/config.json\n原因：Permission denied (os error 13)"
        );
    }

    #[test]
    fn serialize_error_message_is_verbatim() {
        let err = ConfigError::Serialize {
            detail: "serializer error".to_owned(),
        };

        assert_eq!(
            err.to_string(),
            "序列化配置文件失败\n原因：serializer error"
        );
    }
}
