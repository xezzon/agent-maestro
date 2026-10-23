//! 保护态：配置文件损坏或版本不受支持时，存储拒绝一切读写。
//!
//! 保护态是「用户手工改坏了配置文件」这一事实的载体：它让「读不到配置」不再
//! 只是 `Result` 里的一个错误值，而是一个有名字的状态——读写两条路径都经由
//! [`ProtectedState::config`]，因此不可能只在一侧失效。

use super::schema::{Config, StoreError};

/// 存储的内存状态：可用的配置，或「不可用」的原因。
#[derive(Debug)]
pub(super) enum ProtectedState {
    /// 配置可用。
    Ready(Config),
    /// 配置文件不可用：带着原因，读取与写入一律拒绝，绝不静默重建或覆盖原文件。
    Unavailable(StoreError),
}

impl ProtectedState {
    /// 当前配置；处于保护态时按值返回原因。
    pub(super) fn config(&self) -> Result<&Config, StoreError> {
        match self {
            Self::Ready(config) => Ok(config),
            Self::Unavailable(reason) => Err(reason.clone()),
        }
    }

    /// 进入保护态：带着原因，不再提供配置。
    pub(super) fn unavailable(reason: StoreError) -> Self {
        Self::Unavailable(reason)
    }
}
