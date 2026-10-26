//! 偏好存储的错误：偏好文件本身的读取、序列化与落盘。
//!
//! 与 [`crate::errors::ConfigError`] 是两件事、也不共享状态机：配置损坏要
//! 「拒写」（ADR 0001：坏文件拒绝写入，绝不静默覆盖），偏好损坏要「兜底并
//! 重写」（ADR 0023）。一套状态机同时表达不了这两条相反的策略，故偏好有
//! 自己的错误域。

use std::{
    fmt::{self, Display},
    path::{Path, PathBuf},
};

use crate::errors::LockPoisoned;

/// 偏好存储不可用时对用户可见的原因。
const STORE_LOCK_POISONED: &str = "偏好存储不可用";

/// 偏好文件本身的错误：读取、序列化与落盘。
#[derive(Debug, Clone)]
pub(crate) enum PreferencesError {
    /// 读写偏好文件时发生 IO 错误。
    Io { path: PathBuf, detail: String },
    /// 偏好读得出来，却写不出合法的 JSON（序列化失败）。
    ///
    /// 与 IO 失败是两件事：文件没坏、也不是写不动，而是内容写不成 JSON——
    /// 混进 `Io` 会让报错指向错误的修复方向（去查磁盘权限，而不是查内容）。
    Serialize { detail: String },
    /// 偏好文件的版本不被当前应用支持。
    ///
    /// 那可能是更新版本写下的文件：覆盖它等于用旧版销毁新版的数据，故存储层
    /// 拒写（ADR 0023）。
    UnsupportedVersion {
        path: PathBuf,
        /// 文件里的版本取值（原样带进文案，便于用户对照文件）。
        found: String,
        /// 当前应用支持的版本。
        supported: u32,
    },
    /// 偏好存储的锁不可用（其它命令持锁期间 panic 毒化了锁）。
    Unavailable,
}

impl PreferencesError {
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

    pub(crate) fn unsupported_version(
        path: &Path,
        found: impl Into<String>,
        supported: u32,
    ) -> Self {
        Self::UnsupportedVersion {
            path: path.to_path_buf(),
            found: found.into(),
            supported,
        }
    }
}

impl From<LockPoisoned> for PreferencesError {
    fn from(_: LockPoisoned) -> Self {
        Self::Unavailable
    }
}

impl Display for PreferencesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreferencesError::Io { path, detail } => {
                write!(f, "读写偏好文件失败：{}\n原因：{detail}", path.display())
            }
            PreferencesError::Serialize { detail } => {
                write!(f, "序列化偏好失败\n原因：{detail}")
            }
            PreferencesError::UnsupportedVersion {
                path,
                found,
                supported,
            } => write!(
                f,
                "偏好文件版本不受支持：{}\n文件中的 version 为 {found}，当前应用仅支持 {supported}。\n该文件可能由更新版本写出，Maestro 不会覆盖它；请自行处理该文件后重启应用。",
                path.display()
            ),
            PreferencesError::Unavailable => write!(f, "{STORE_LOCK_POISONED}"),
        }
    }
}

impl std::error::Error for PreferencesError {}
