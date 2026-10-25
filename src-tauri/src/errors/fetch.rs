//! https 拉取的错误：网络、状态码、响应体体积。

use std::fmt::{self, Display};

/// https 拉取的错误：网络、状态码、响应体体积。
#[derive(Debug, Clone)]
pub(crate) enum FetchError {
    /// 客户端初始化失败（TLS 后端不可用）。
    ClientInit { detail: String },
    /// 请求失败（连接、超时或 4xx/5xx 状态码）。
    Request { url: String, detail: String },
    /// 读取响应体失败。
    ReadResponse { url: String, detail: String },
    /// 响应超过体积上限（不静默截断）。
    TooLarge { url: String, limit_mib: u64 },
}

impl FetchError {
    pub(crate) fn client_init(detail: impl Into<String>) -> Self {
        Self::ClientInit {
            detail: detail.into(),
        }
    }

    pub(crate) fn request(url: &str, detail: impl Into<String>) -> Self {
        Self::Request {
            url: url.to_owned(),
            detail: detail.into(),
        }
    }

    pub(crate) fn read_response(url: &str, detail: impl Into<String>) -> Self {
        Self::ReadResponse {
            url: url.to_owned(),
            detail: detail.into(),
        }
    }

    pub(crate) fn too_large(url: &str, limit_mib: u64) -> Self {
        Self::TooLarge {
            url: url.to_owned(),
            limit_mib,
        }
    }
}

impl Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::ClientInit { detail } => {
                write!(f, "初始化 https 客户端失败：{detail}")
            }
            FetchError::Request { url, detail } => write!(f, "请求 {url} 失败：{detail}"),
            FetchError::ReadResponse { url, detail } => {
                write!(f, "读取 {url} 的响应失败：{detail}")
            }
            FetchError::TooLarge { url, limit_mib } => {
                write!(f, "{url} 的响应超过 {limit_mib} MiB 上限")
            }
        }
    }
}

impl std::error::Error for FetchError {}
