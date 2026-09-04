use std::collections::BTreeMap;

/// 系统密钥链的抽象（见 ADR 0002）：真实密钥只入密钥链，配置只存 `secret://` 引用。
///
/// 调用方以密钥引用（配置中的不透明 `secret://` URI）寻址；实现负责把引用
/// 解析为密钥链条目，密钥种类与 account 布局（如 `provider/<slug>/api_key`）
/// 属于实现细节，对调用方透明。本票以内存 fake 接入应用，
/// 基于 `keyring` crate 的真实实现见后续票。
pub trait Keychain {
    /// 清除密钥引用指向的条目。
    ///
    /// 条目不存在时同样成功（幂等）。失败不携带敏感信息，返回的说明面向最终用户。
    fn clear(&mut self, secret_reference: &str) -> Result<(), KeychainError>;
}

/// 密钥链操作失败的原因说明（面向最终用户）。
#[derive(Debug, Clone)]
pub struct KeychainError {
    detail: String,
}

impl KeychainError {
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

/// 内存 fake：先于真实实现接入应用（Linux Secret Service 在 CI 中不可靠，
/// 测试一律注入本 fake）。
///
/// 按引用原样索引条目，不校验引用格式。`entries` 与 `fail_clear` 主要供测试
/// 搭建场景：置 `fail_clear` 为 true 即可模拟密钥链不可用，
/// 验证「清除失败不阻塞删除」。
#[derive(Debug, Default)]
pub struct FakeKeychain {
    /// 密钥引用 -> 密钥真值。
    pub entries: BTreeMap<String, String>,
    /// 置为 true 后所有清除操作失败。
    pub fail_clear: bool,
}

impl Keychain for FakeKeychain {
    fn clear(&mut self, secret_reference: &str) -> Result<(), KeychainError> {
        if self.fail_clear {
            return Err(KeychainError::new("模拟密钥链不可用"));
        }
        self.entries.remove(secret_reference);
        Ok(())
    }
}
