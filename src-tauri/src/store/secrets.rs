//! Provider 凭证的写入：密钥链优先、配置只留引用。
//!
//! 三态契约（不变 / 清除 / 覆盖）在这一处落地，规则只有一份：
//!
//! - **不变**：完全不碰密钥链，字段沿用旧值；
//! - **清除**：先删条目（不存在算成功），再落空串；
//! - **覆盖**：先写密钥链，再落规范引用。
//!
//! 写入顺序永远是「先密钥链、后配置」：配置里只留引用，真值只进密钥链
//! （ADR 0002）。反过来（先落配置再写密钥链）会让配置指向一个不存在的条目，
//! 密钥就此丢失。
//!
//! 配置落盘失败时用旧值回滚密钥链：变更前有条目就写回原值，变更前没有条目
//! 就删掉新写的那个。回滚本身也失败时，两次失败一并点名（见
//! [`ProviderError::SecretRollback`]）——那时配置没改动，密钥链却可能与配置
//! 不一致，用户得知道两处都要看一眼。

use crate::{
    errors::{ProviderError, SecretError},
    secret::{SecretRef, SecretStore, SecretText},
};

/// 一次凭证写入的结果：字段的最终形态 + 密钥链的回滚动作。
pub(super) struct SecretWrite {
    /// 字段最终形态：未设置（落空串）或规范引用。
    pub(super) value: SecretText,
    /// 回滚动作：把密钥链恢复到本次变更之前的状态。
    pub(super) rollback: Option<Rollback>,
}

impl SecretWrite {
    /// 未设置：不碰密钥链，也没有东西要回滚。
    pub(super) fn unset() -> Self {
        Self {
            value: SecretText::unset(),
            rollback: None,
        }
    }

    /// 沿用旧值：不碰密钥链，也没有东西要回滚。
    pub(super) fn keep(existing: &SecretText) -> Self {
        Self {
            value: existing.clone(),
            rollback: None,
        }
    }
}

/// 回滚动作：把条目恢复到本次变更之前的状态。
pub(super) struct Rollback {
    /// 被指向的条目。
    reference: SecretRef,
    /// 变更前的条目值；`None` 即变更前没有条目（回滚即删掉新值）。
    previous: Option<String>,
}

impl Rollback {
    /// 恢复：变更前有条目就写回原值，否则删掉新写的条目。
    pub(super) fn apply(&self, secrets: &dyn SecretStore) -> Result<(), SecretError> {
        match &self.previous {
            Some(value) => secrets.set(&self.reference, value),
            None => secrets.delete(&self.reference),
        }
    }
}

/// 本 Provider 的规范密钥引用。
///
/// slug 不合法即拒绝：引用与 slug 绑定（GLOSSARY「密钥引用」），构造不出
/// 引用就没有地方放真值——宁可报错，也不把真值退回配置文件（ADR 0002）。
pub(super) fn canonical(slug: &str) -> Result<SecretRef, ProviderError> {
    SecretRef::for_provider(slug).map_err(|_| ProviderError::invalid_slug(slug))
}

/// 覆盖：先写密钥链，再落规范引用。
pub(super) fn overwrite(
    reference: &SecretRef,
    value: &str,
    secrets: &dyn SecretStore,
) -> Result<SecretWrite, SecretError> {
    let rollback = mutate(reference, || secrets.set(reference, value), secrets)?;
    Ok(SecretWrite {
        value: SecretText::referenced(reference.clone()),
        rollback: Some(rollback),
    })
}

/// 清除：先删条目（不存在算成功），再落空串。
pub(super) fn clear(
    reference: &SecretRef,
    secrets: &dyn SecretStore,
) -> Result<SecretWrite, SecretError> {
    let rollback = mutate(reference, || secrets.delete(reference), secrets)?;
    Ok(SecretWrite {
        value: SecretText::unset(),
        rollback: Some(rollback),
    })
}

/// 删除 Provider 时的级联：仅当字段是引用时才删密钥链条目。
///
/// 明文与未设置直接删记录，不碰密钥链——只有引用才指向密钥链条目，别的
/// 形态压根没有条目可删。
pub(super) fn cascade(
    existing: &SecretText,
    secrets: &dyn SecretStore,
) -> Result<Option<Rollback>, SecretError> {
    let Some(reference) = existing.reference() else {
        return Ok(None);
    };
    Ok(Some(mutate(
        reference,
        || secrets.delete(reference),
        secrets,
    )?))
}

/// 变更条目：先读旧值（回滚要用），再执行动作。
///
/// 旧值在动作**之前**读：配置落盘失败时，拿它把条目恢复回去（变更前没有条目
/// 就删掉新写的那个）。
fn mutate(
    reference: &SecretRef,
    action: impl FnOnce() -> Result<(), SecretError>,
    secrets: &dyn SecretStore,
) -> Result<Rollback, SecretError> {
    let previous = secrets.get(reference)?;
    action()?;
    Ok(Rollback {
        reference: reference.clone(),
        previous,
    })
}
