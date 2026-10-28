//! 内存密钥链：测试替身。
//!
//! 它只存在于测试构建里（`#[cfg(test)]`）：真值在生产路径上永远落在系统
//! 密钥链，内存实现**绝不**作为回退——那等于把真值留在进程内存里，用户
//! 以为存进了密钥链，实际没有（ADR 0002）。
//!
//! 替身要能造出真实后端造得出的情形：条目不存在、密钥链不可用、操作失败。
//! 故它支持预置条目、脚本化失败与调用记录。

use std::{
    collections::{BTreeMap, VecDeque},
    sync::Mutex,
};

use crate::{
    errors::SecretError,
    secret::{
        reference::SecretRef,
        store::{SecretOp, SecretStore},
    },
};

/// 一次调用：动作与被指向的 account。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SecretCall {
    /// 动作。
    pub(crate) op: SecretOp,
    /// 密钥链条目的 account（`provider/<slug>/api_key`）。
    pub(crate) account: String,
}

/// 内存密钥链：可预置条目、可脚本化失败、可断言调用后的内容。
pub(crate) struct MemorySecrets {
    state: Mutex<State>,
}

/// 替身的全部状态：一把锁护住，模拟后端的原子性。
#[derive(Default)]
struct State {
    /// 条目：account → 真值。
    entries: BTreeMap<String, String>,
    /// 脚本化的失败：按动作排队，调用时按序取用。
    failures: BTreeMap<SecretOp, VecDeque<SecretError>>,
    /// 调用记录：断言「某个动作压根没被调用」用。
    calls: Vec<SecretCall>,
}

impl MemorySecrets {
    /// 空的内存密钥链。
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(State::default()),
        }
    }

    /// 预置条目：让替身从「已经有一条」的状态起步。
    pub(crate) fn with_entry(reference: &SecretRef, value: &str) -> Self {
        let store = Self::new();
        store.insert(reference, value);
        store
    }

    /// 脚本化：下一个该动作的调用失败（多次调用则按序依次失败）。
    pub(crate) fn fail_next(&self, op: SecretOp, error: SecretError) {
        let mut state = self.lock();
        state.failures.entry(op).or_default().push_back(error);
    }

    /// 条目内容：断言用。
    pub(crate) fn entry(&self, reference: &SecretRef) -> Option<String> {
        self.lock().entries.get(&reference.account()).cloned()
    }

    /// 全部条目：断言用（account → 真值）。
    pub(crate) fn entries(&self) -> BTreeMap<String, String> {
        self.lock().entries.clone()
    }

    /// 调用记录：断言「某个动作压根没被调用」用。
    pub(crate) fn calls(&self) -> Vec<SecretCall> {
        self.lock().calls.clone()
    }

    /// 取锁：锁被毒化说明先前的断言在锁内 panic 过。
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .expect("内存密钥链的锁被毒化：先前的断言在锁内 panic 过")
    }

    /// 写入条目（预置用）。
    fn insert(&self, reference: &SecretRef, value: &str) {
        self.lock()
            .entries
            .insert(reference.account(), value.to_owned());
    }

    /// 记一次调用，并按脚本决定这次是否失败。
    fn record(&self, op: SecretOp, account: &str) -> Result<(), SecretError> {
        let mut state = self.lock();
        state.calls.push(SecretCall {
            op,
            account: account.to_owned(),
        });
        match state.failures.get_mut(&op).and_then(VecDeque::pop_front) {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl SecretStore for MemorySecrets {
    fn set(&self, reference: &SecretRef, value: &str) -> Result<(), SecretError> {
        let account = reference.account();
        self.record(SecretOp::Write, &account)?;
        self.lock().entries.insert(account, value.to_owned());
        Ok(())
    }

    fn get(&self, reference: &SecretRef) -> Result<Option<String>, SecretError> {
        let account = reference.account();
        self.record(SecretOp::Read, &account)?;
        Ok(self.lock().entries.get(&account).cloned())
    }

    fn delete(&self, reference: &SecretRef) -> Result<(), SecretError> {
        let account = reference.account();
        self.record(SecretOp::Delete, &account)?;
        self.lock().entries.remove(&account);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::SecretCause;

    fn reference() -> SecretRef {
        SecretRef::for_provider("openrouter").unwrap()
    }

    /// 预置的条目读得回来。
    #[test]
    fn preset_entry_is_readable() {
        let store = MemorySecrets::with_entry(&reference(), "sk-value");

        assert_eq!(store.entry(&reference()), Some("sk-value".to_owned()));
        assert_eq!(
            store.get(&reference()).unwrap(),
            Some("sk-value".to_owned())
        );
    }

    /// 未预置的条目读作「没有值」，不是错误。
    #[test]
    fn missing_entry_reads_as_none() {
        let store = MemorySecrets::new();

        assert_eq!(store.get(&reference()).unwrap(), None);
        assert_eq!(store.entry(&reference()), None);
    }

    /// 写入覆盖既有条目。
    #[test]
    fn set_overwrites_the_entry() {
        let store = MemorySecrets::with_entry(&reference(), "old");

        store.set(&reference(), "new").unwrap();

        assert_eq!(store.entry(&reference()), Some("new".to_owned()));
    }

    /// 删除后条目不在了；再删一次仍算成功（幂等）。
    #[test]
    fn delete_is_idempotent() {
        let store = MemorySecrets::with_entry(&reference(), "sk-value");

        store.delete(&reference()).unwrap();

        assert_eq!(store.entry(&reference()), None);
        store.delete(&reference()).expect("条目不存在视作成功");
    }

    /// 脚本化的失败按动作生效，且只影响被指定的动作。
    #[test]
    fn scripted_failure_fires_on_the_next_call_of_that_operation() {
        let store = MemorySecrets::new();
        store.fail_next(
            SecretOp::Read,
            SecretError::Read {
                cause: SecretCause::Unavailable {
                    reason: "密钥链被锁着".to_owned(),
                },
            },
        );

        let error = store.get(&reference()).unwrap_err();

        assert_eq!(
            error,
            SecretError::Read {
                cause: SecretCause::Unavailable {
                    reason: "密钥链被锁着".to_owned()
                }
            }
        );
        assert_eq!(store.get(&reference()).unwrap(), None, "失败只发生一次");
    }

    /// 失败的动作不影响其它动作。
    #[test]
    fn scripted_failure_is_scoped_to_its_operation() {
        let store = MemorySecrets::new();
        store.fail_next(
            SecretOp::Write,
            SecretError::Write {
                cause: SecretCause::Rejected {
                    reason: "写入失败".to_owned(),
                },
            },
        );

        assert!(store.set(&reference(), "sk-value").is_err());

        assert_eq!(
            store.get(&reference()).unwrap(),
            None,
            "读取不受写入失败影响"
        );
    }

    /// 调用记录：断言「压根没调用过」用。
    #[test]
    fn calls_are_recorded_per_operation() {
        let store = MemorySecrets::new();

        store.set(&reference(), "sk-value").unwrap();
        store.get(&reference()).unwrap();

        let calls = store.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].op, SecretOp::Write);
        assert_eq!(calls[0].account, "provider/openrouter/api_key");
        assert_eq!(calls[1].op, SecretOp::Read);
    }
}
