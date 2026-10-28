//! 密钥链端口：读、写、删三个动作。

use crate::{
    errors::{SecretCause, SecretError},
    secret::reference::SecretRef,
};

/// 密钥链动作：写、读、删。
///
/// 端口的三个方法、失败的归类与测试替身的调用记录共用它——同一组动作不必
/// 各写一份枚举（写路径叫「写」、替身里也叫「写」，两处对得上）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SecretOp {
    /// 写入（`set`）：真值写进密钥链。
    Write,
    /// 读取（`get`）：从密钥链取回真值。
    Read,
    /// 删除（`delete`）：删掉密钥链条目。
    Delete,
}

impl SecretOp {
    /// 把成因折进本动作的失败变体。
    pub(crate) fn into_error(self, cause: SecretCause) -> SecretError {
        match self {
            SecretOp::Write => SecretError::Write { cause },
            SecretOp::Read => SecretError::Read { cause },
            SecretOp::Delete => SecretError::Delete { cause },
        }
    }
}

/// 密钥链端口：真值的唯一落点。
///
/// 端口只说「条目在哪儿」与「做什么」，不说「存到哪个后端」——生产实现是
/// 系统密钥链（[`crate::secret::KeyringSecrets`]），测试替身是内存实现
/// （`MemorySecrets`）。
///
/// 契约：
///
/// - `get` 遇条目不存在返回 `Ok(None)`，**不**是错误——「没配」与「坏了」
///   必须分得开；
/// - `delete` 遇条目不存在视作成功（幂等）——清除与删除都靠这条收敛；
/// - 错误只描述失败，不携带密钥值。
///
/// trait 对象满足 `Send + Sync`：实现以 `Arc<dyn SecretStore>` 注入，可跨
/// 线程共享（实现内部自行把对同一后端的调用串行化）。
pub(crate) trait SecretStore: Send + Sync {
    /// 写入真值：同一引用重复写入即覆盖。
    fn set(&self, reference: &SecretRef, value: &str) -> Result<(), SecretError>;

    /// 读取真值；条目不存在即 `None`。
    fn get(&self, reference: &SecretRef) -> Result<Option<String>, SecretError>;

    /// 删除条目；条目不存在视作成功。
    fn delete(&self, reference: &SecretRef) -> Result<(), SecretError>;
}
