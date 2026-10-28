//! 系统密钥链：唯一的生产实现。
//!
//! 后端由 `keyring` 提供，按平台分依赖（见 `Cargo.toml`）：macOS 走
//! `apple-native`（Keychain）、Windows 走 `windows-native`（凭据管理器）、
//! Linux 走 `sync-secret-service`（Secret Service）。不支持的编译目标没有
//! 这份依赖，编译期即失败（见 [`crate::secret`] 的 `compile_error!`）。

use std::sync::{Mutex, MutexGuard};

use crate::{
    errors::{SecretCause, SecretError},
    secret::{
        reference::SecretRef,
        store::{SecretOp, SecretStore},
    },
};

/// 串行化全部调用的闸门。
///
/// **进程级**而非实例级：Windows 凭据管理器不保证跨线程串行，那是平台级
/// 的约束，故闸门也得是平台级——每个实例各带一把锁的话，多一个实例就多
/// 一份「看起来串行、其实不串行」的错觉。
static GATE: Mutex<()> = Mutex::new(());

/// 系统密钥链：真值的唯一持久落点。
///
/// 条目的定位方式：service 固定为命名空间（见 [`SecretRef::service`]），
/// account 为 `provider/<slug>/api_key`（见 [`SecretRef::account`]）——与
/// GLOSSARY「密钥链」的约定一致，条目因此可按 Provider 寻址。
///
/// 全部调用经 [`GATE`] 串行化：临界区里只有一次平台调用，不夹带别的事。
pub(crate) struct KeyringSecrets;

impl KeyringSecrets {
    /// 新建一个系统密钥链实例。
    pub(crate) fn new() -> Self {
        Self
    }

    /// 取临界区：锁被毒化说明先前的调用在锁内 panic 过，如实报错而不放行。
    fn lock(op: SecretOp) -> Result<MutexGuard<'static, ()>, SecretError> {
        GATE.lock().map_err(|_| {
            op.into_error(SecretCause::Unavailable {
                reason: "密钥链的调用锁已被毒化".to_owned(),
            })
        })
    }
}

impl SecretStore for KeyringSecrets {
    fn set(&self, reference: &SecretRef, value: &str) -> Result<(), SecretError> {
        let _guard = Self::lock(SecretOp::Write)?;
        let entry = keyring::Entry::new(reference.service(), &reference.account())
            .map_err(|error| classify(&error, SecretOp::Write))?;
        entry
            .set_password(value)
            .map_err(|error| classify(&error, SecretOp::Write))?;
        // 只记标识符（slug），绝不记值（ADR 0008：凭证绝不落盘进日志）。
        log::debug!("keychain entry written: slug={}", reference.slug());
        Ok(())
    }

    fn get(&self, reference: &SecretRef) -> Result<Option<String>, SecretError> {
        let _guard = Self::lock(SecretOp::Read)?;
        let entry = keyring::Entry::new(reference.service(), &reference.account())
            .map_err(|error| classify(&error, SecretOp::Read))?;
        match entry
            .get_password()
            .map_err(|error| classify(&error, SecretOp::Read))
        {
            Ok(value) => {
                log::debug!("keychain entry read: slug={}", reference.slug());
                Ok(Some(value))
            }
            // 条目不存在不是失败：读作「没有值」。
            Err(SecretError::Read {
                cause: SecretCause::NoEntry,
            }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn delete(&self, reference: &SecretRef) -> Result<(), SecretError> {
        let _guard = Self::lock(SecretOp::Delete)?;
        let entry = keyring::Entry::new(reference.service(), &reference.account())
            .map_err(|error| classify(&error, SecretOp::Delete))?;
        match entry
            .delete_credential()
            .map_err(|error| classify(&error, SecretOp::Delete))
        {
            Ok(()) => {
                log::debug!("keychain entry deleted: slug={}", reference.slug());
                Ok(())
            }
            // 条目不存在视作成功：删除是幂等的。
            Err(SecretError::Delete {
                cause: SecretCause::NoEntry,
            }) => Ok(()),
            Err(error) => Err(error),
        }
    }
}

/// 把 `keyring` 的错误归类：条目不存在单列，密钥链不可用单列，其余按平台拒绝处理。
///
/// 「条目不存在」与「密钥链不可用」必须分开：前者是正常状态（读作没有值、
/// 删作成功），后者是故障（调用方要如实报错，绝不回退为明文，ADR 0002）。
///
/// 「密钥链不可用」取的是「后端用不了」这一类：平台明说存储不可访问
/// （`NoStorageAccess`：条目库被锁、被访问规则拒绝），或平台调用整体失败
/// （`PlatformFailure`：Linux 上连不上 D-Bus 会话、Secret Service 缺席即
/// 此类）。余下的是平台拒绝了这次请求（值编码不对、属性超长、同名条目多条）。
fn classify(error: &keyring::Error, op: SecretOp) -> SecretError {
    let cause = match error {
        keyring::Error::NoEntry => SecretCause::NoEntry,
        keyring::Error::NoStorageAccess(reason) => SecretCause::Unavailable {
            reason: reason.to_string(),
        },
        keyring::Error::PlatformFailure(reason) => SecretCause::Unavailable {
            reason: reason.to_string(),
        },
        other => SecretCause::Rejected {
            reason: reason_of(other),
        },
    };
    op.into_error(cause)
}

/// 失败原因：只描述平台侧的失败，**不含密钥值**。
///
/// `keyring::Error` 的 `Debug` 会连同载荷一起打印（`BadEncoding` 带着取回
/// 的密码字节、`Ambiguous` 带着条目内容），故此处逐个变体取安全字段，绝不
/// 整体 `Debug` 化。
fn reason_of(error: &keyring::Error) -> String {
    match error {
        keyring::Error::BadEncoding(_) => "密钥链中的条目不是 UTF-8 文本".to_owned(),
        keyring::Error::TooLong(attribute, limit) => {
            format!("属性 {attribute} 超出平台长度上限 {limit}")
        }
        keyring::Error::Invalid(attribute, reason) => {
            format!("属性 {attribute} 非法：{reason}")
        }
        keyring::Error::Ambiguous(items) => format!("匹配到 {} 条同名条目", items.len()),
        // `keyring::Error` 是 non_exhaustive：未知的变体不得当成已知成因。
        _ => "密钥链返回未知错误".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// 生产实现可跨线程共享：端口对象满足 `Send + Sync`。
    #[test]
    fn secret_store_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Arc<dyn SecretStore>>();
        assert_send_sync::<KeyringSecrets>();
    }

    /// 条目定位方式钉住：service 是固定命名空间，account 是
    /// `provider/<slug>/api_key`——条目因此可按 Provider 寻址。
    #[test]
    fn entries_are_addressed_by_service_and_account() {
        let reference = SecretRef::for_provider("openrouter").unwrap();

        assert_eq!(reference.service(), "io.github.xezzon.agent-maestro");
        assert_eq!(reference.account(), "provider/openrouter/api_key");
    }

    /// 失败原因不带密钥值：`keyring::Error` 的载荷里可能有密码字节，故
    /// 只取安全字段。
    #[test]
    fn reasons_never_carry_the_secret() {
        let error = keyring::Error::BadEncoding(b"sk-canary-4b7e".to_vec());

        let reason = reason_of(&error);

        assert!(
            !reason.contains("sk-canary-4b7e"),
            "失败原因不得携带密钥值：{reason}"
        );
    }

    /// 归类：条目不存在与密钥链不可用分得开；后端用不了（连不上）算不可用。
    #[test]
    fn classification_separates_missing_from_unavailable() {
        let missing = classify(&keyring::Error::NoEntry, SecretOp::Read);
        assert_eq!(
            missing,
            SecretError::Read {
                cause: SecretCause::NoEntry
            }
        );

        let unavailable = classify(
            &keyring::Error::PlatformFailure("no session bus".into()),
            SecretOp::Write,
        );
        assert_eq!(
            unavailable,
            SecretError::Write {
                cause: SecretCause::Unavailable {
                    reason: "no session bus".to_owned()
                }
            }
        );
    }
}
