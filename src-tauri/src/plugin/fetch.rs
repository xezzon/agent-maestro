//! https 拉取的注入点：URL → 字节（本设计唯一的新缝，见 ADR 0006）。
//!
//! 宿主不感知 GitHub，只做纯 https 下载；测试以替身注入，生产实现是薄适配器。

use std::{io::Read, time::Duration};

use reqwest::blocking::Client;

/// 拉取插件来源的字节。
pub trait Fetcher: Send + Sync {
    /// 下载 `url` 指向的字节；失败以人类可读原因传播。
    fn fetch(&self, url: &str) -> Result<Vec<u8>, String>;
}

/// 单次响应的体积上限：wasm 产物可达数十 MB，超出即视为异常来源而非静默截断。
const BODY_LIMIT: u64 = 64 * 1024 * 1024;

/// 建连（含 TLS 握手）时限：主机不可达时快速失败，而非让用户干等。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// 每次读写的时限。下载总耗时取决于用户的链路（实测有 ~50KB/s 的慢链路），
/// 故不设总时限，只用来兜住彻底无响应的服务器。
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// 生产实现：同步 reqwest（rustls + 平台信任库）。
///
/// 仅接受 https（`https_only`），默认跟随重定向（上限 10 跳）——GitHub Release
/// 的资产下载会重定向到对象存储域名。
pub struct HttpFetcher {
    /// 客户端初始化失败（TLS 后端不可用）时保留原因：插件进错误态即可，
    /// 不必让整个应用起不来。
    client: Result<Client, String>,
}

impl HttpFetcher {
    pub fn new() -> Self {
        let client = Client::builder()
            .https_only(true)
            .user_agent(concat!("agent-maestro/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(READ_TIMEOUT)
            .build()
            .map_err(|e| format!("初始化 https 客户端失败：{e}"));
        Self { client }
    }
}

impl Default for HttpFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetcher for HttpFetcher {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, String> {
        let client = self.client.as_ref().map_err(Clone::clone)?;
        // 4xx/5xx 不是 reqwest 的默认错误，必须显式转错误：
        // 否则 404 页面会被当作 manifest 去解析，报出误导性的校验错误。
        let response = client
            .get(url)
            .send()
            .map_err(|e| format!("请求 {url} 失败：{e}"))?
            .error_for_status()
            .map_err(|e| format!("请求 {url} 失败：{e}"))?;

        let mut body = Vec::new();
        response
            .take(BODY_LIMIT + 1)
            .read_to_end(&mut body)
            .map_err(|e| format!("读取 {url} 的响应失败：{e}"))?;
        if body.len() as u64 > BODY_LIMIT {
            return Err(format!(
                "{url} 的响应超过 {} MiB 上限",
                BODY_LIMIT / (1024 * 1024)
            ));
        }
        Ok(body)
    }
}
