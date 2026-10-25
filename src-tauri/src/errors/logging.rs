//! 日志设施的错误：文件目标的写探测失败。

use std::{
    fmt::{self, Display},
    path::{Path, PathBuf},
};

/// 日志文件目标的写探测失败；调用方据此降级为只装 Stdout。
#[derive(Debug, Clone)]
pub(crate) enum LoggingError {
    CreateDir { path: PathBuf, detail: String },
    OpenFile { path: PathBuf, detail: String },
}

impl LoggingError {
    pub(crate) fn create_dir(path: &Path, detail: impl Into<String>) -> Self {
        Self::CreateDir {
            path: path.to_path_buf(),
            detail: detail.into(),
        }
    }

    pub(crate) fn open_file(path: &Path, detail: impl Into<String>) -> Self {
        Self::OpenFile {
            path: path.to_path_buf(),
            detail: detail.into(),
        }
    }
}

impl Display for LoggingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoggingError::CreateDir { path, detail } => {
                write!(f, "create {}: {detail}", path.display())
            }
            LoggingError::OpenFile { path, detail } => {
                write!(f, "open {}: {detail}", path.display())
            }
        }
    }
}

impl std::error::Error for LoggingError {}
