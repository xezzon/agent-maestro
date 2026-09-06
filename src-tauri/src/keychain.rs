use std::collections::BTreeMap;

/// 密钥引用的 URI scheme（见 ADR 0002）。
///
/// 配置文件中的「密钥引用」一律以 `secret://` 开头，便于与 http(s) 等其它
/// URI 区分；引用只由 [`Secret::reference`] 统一拼出，命令层只接受
/// `{ slug, value }` 形态的 `api_key` 负载，避免误把密钥真值当作引用保存。
pub const SECRET_SCHEME: &str = "secret://";

/// 密钥链在系统密钥服务中的命名空间（`secret://` URI 的 service 段）。
///
/// 与 ADR 0002 / 0003 一致：`secret://io.github.xezzon.agent-maestro/...`。
/// 不同服务在系统密钥链里相互隔离，避免与同机其它应用冲突。
pub const NAMESPACE: &str = "io.github.xezzon.agent-maestro";

/// 一次密钥操作的意图（见 ADR 0002 的三态更新契约）：`account` 定位命名空间下的
/// 密钥链条目，`value` 携带待写入的真值。
///
/// `value` 为 `Some` 表示覆盖写入；为 `None` 表示不携带真值——create 仅登记引用
/// （配置侧只落 [`Secret::reference`] 拼出的 `secret://` URI），update 时由命令层
/// 按更新契约解释为清除。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Secret {
    /// 密钥链条目地址（相对命名空间），如 `provider/<slug>/api_key`。
    account: String,
    /// 待写入的密钥真值；`None` 表示仅登记引用。
    value: Option<String>,
}

impl Secret {
    pub fn new(account: String, value: Option<String>) -> Self {
        Self { account, value }
    }

    /// 拼出该条目的密钥引用 URI，即配置中保存的形态。
    pub fn reference(&self) -> String {
        let account = &self.account;
        format!("{SECRET_SCHEME}{NAMESPACE}/{account}")
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    const REFERENCE: &str = "secret://io.github.xezzon.agent-maestro/provider/ollama/api_key";

    #[test]
    fn reference_uris_are_namespaced() {
        let secret = Secret::new("provider/ollama/api_key".to_owned(), None);

        assert_eq!(secret.reference(), REFERENCE);
    }

    #[test]
    fn clear_removes_entry_and_is_idempotent() {
        let mut keychain = FakeKeychain::default();
        keychain
            .entries
            .insert(REFERENCE.to_owned(), "sk-test".to_owned());

        keychain.clear(REFERENCE).unwrap();
        keychain.clear(REFERENCE).unwrap();

        assert!(
            !keychain.entries.contains_key(REFERENCE),
            "清除不存在的条目同样成功（幂等）"
        );
    }

    #[test]
    fn clear_failure_returns_user_facing_error() {
        let mut keychain = FakeKeychain {
            fail_clear: true,
            ..Default::default()
        };

        let err = keychain.clear(REFERENCE).unwrap_err();

        assert!(!err.detail().is_empty(), "失败说明面向最终用户，不能为空");
    }
}
