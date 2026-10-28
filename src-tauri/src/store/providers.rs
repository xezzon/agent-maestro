//! Provider 的领域规则：参与投影的筛选、落盘前的一致性校验与启停。
//!
//! 落盘前的一致性校验（provider↔variables，ADR 0015）也在这里：写路径统一
//! 经由本模块，校验规则只有一份——存储层与投影层因此不会各判各的。
//!
//! 新增与删除记录不在本模块：它们要先把凭证落到密钥链、再落配置，两步之间
//! 还要能回滚（见 [`super::secrets`]），编排归 [`super::Store`]。

use std::collections::BTreeMap;

use crate::{
    config::{Change, Config},
    errors::{ProviderError, ProviderInterpolationError},
    provider::Provider,
    variables::Variables,
};

/// 挑出参与投影的 Provider（#94）：`enabled == false` 者整体不参与——既不写入
/// 插件，也不做占位符插值，因此其字段里的占位符错误不会牵连整个插件失败。
///
/// 这是存储层的领域规则（不是传输层的）：[`super::Store::enabled_providers`]
/// 是它唯一的调用方，`apply_providers` 经由那次查询取数，不再各自复述一遍
/// 筛选口径。模块外不再可见——规则只从 `Store` 的语义化查询走。
pub(super) fn select_enabled_providers(
    providers: &BTreeMap<String, Provider>,
) -> BTreeMap<String, Provider> {
    providers
        .iter()
        .filter(|(_, provider)| provider.enabled)
        .map(|(slug, provider)| (slug.clone(), provider.clone()))
        .collect()
}

/// 启用/禁用单个 Provider（独立于编辑表单的开关）。
///
/// 幂等：状态未变即不写盘。切换不改变占位符文本，而存储写出的每条记录都已
/// 通过落盘一致性校验（新增/更新/整表换变量均不豁免禁用项），故切换无需再次
/// 校验插值。
pub(super) fn set_enabled(
    config: &mut Config,
    slug: &str,
    enabled: bool,
) -> Result<Change<()>, ProviderError> {
    let provider = config
        .providers
        .get_mut(slug)
        .ok_or_else(|| ProviderError::missing_slug(slug))?;
    if provider.enabled == enabled {
        return Ok(Change::Unchanged(()));
    }
    provider.enabled = enabled;
    Ok(Change::Changed(()))
}

/// Provider 落盘前的插值校验（provider↔variables 一致性，ADR 0015）。
///
/// 全部 Provider（含禁用者）都须对当前变量表可解析：落盘内容始终是自洽的，
/// 禁用只是一种投影时的结构过滤，不改变存储不变式。
/// 新增/更新/整表换变量共用这一条规则。
///
/// 校验失败由调用方包成各仓储自己的错误类型（[`ProviderError::Interpolation`] /
/// `VariableError::Interpolation`）：与投影时（`interpolate_providers`）同因同报、
/// 同款文案，只是提前到落盘前——包裹领域层的同一个错误类型，句式只有一份。
pub(super) fn validate_provider(
    slug: &str,
    provider: &Provider,
    variables: &Variables,
) -> Result<(), ProviderInterpolationError> {
    provider
        .interpolate(variables)
        .map_err(|error| ProviderInterpolationError {
            slug: slug.to_owned(),
            error,
        })?;
    Ok(())
}
