//! Provider 仓储：Provider 记录的新增、更新、删除与启停。
//!
//! 落盘前的一致性校验（provider↔variables，ADR 0015）也在这里：写路径统一
//! 经由本模块，校验规则只有一份——存储层与投影层因此不会各判各的。

use std::collections::BTreeMap;

use crate::{
    provider::{FieldError, Provider},
    variables::Variables,
};

use super::schema::{Change, Config, StoreError};

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

/// 新建一条 Provider（slug 唯一）。
///
/// 落盘前对当前变量表插值一次：占位符引用必须可解析（变量已定义，或带
/// 内联默认值），语法错误同样拦截——provider↔variables 一致性；
/// 禁用状态不影响该校验。
pub(super) fn create(
    config: &mut Config,
    slug: &str,
    provider: Provider,
) -> Result<Change<()>, StoreError> {
    if config.providers.contains_key(slug) {
        return Err(StoreError::duplicate_slug(slug));
    }
    validate_provider(slug, &provider, &config.variables)?;
    config.providers.insert(slug.to_owned(), provider);
    Ok(Change::Changed(()))
}

/// 整包替换指定 Provider 的端点、模型列表与 API Key（明文）。
///
/// slug 不存在时报错，不做 upsert；落盘前对当前变量表插值一次，占位符引用必须
/// 可解析（同 [`create`]，禁用状态不影响校验）。返回被替换的原记录。
pub(super) fn update(
    config: &mut Config,
    slug: &str,
    provider: Provider,
) -> Result<Change<Provider>, StoreError> {
    let original = config
        .providers
        .get(slug)
        .cloned()
        .ok_or_else(|| StoreError::missing_slug(slug))?;
    validate_provider(slug, &provider, &config.variables)?;
    config.providers.insert(slug.to_owned(), provider);
    Ok(Change::Changed(original))
}

/// 删除 Provider，其端点、模型与 API Key 随记录一并移除，不留孤儿数据。
/// 返回被删除的记录。
pub(super) fn delete(config: &mut Config, slug: &str) -> Result<Change<Provider>, StoreError> {
    let removed = config
        .providers
        .remove(slug)
        .ok_or_else(|| StoreError::missing_slug(slug))?;
    Ok(Change::Changed(removed))
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
) -> Result<Change<()>, StoreError> {
    let provider = config
        .providers
        .get_mut(slug)
        .ok_or_else(|| StoreError::missing_slug(slug))?;
    if provider.enabled == enabled {
        return Ok(Change::Unchanged(()));
    }
    provider.enabled = enabled;
    Ok(Change::Changed(()))
}

/// Provider 插值失败 → 存储错误：与投影时（`interpolate_providers`）同因同报、
/// 同款文案，只是提前到落盘前。
fn interpolation_error(slug: &str, error: FieldError) -> StoreError {
    StoreError::Interpolation {
        slug: slug.to_owned(),
        field: error.field,
        reason: error.reason,
    }
}

/// Provider 落盘前的插值校验（provider↔variables 一致性，ADR 0015）。
///
/// 全部 Provider（含禁用者）都须对当前变量表可解析：落盘内容始终是自洽的，
/// 禁用只是一种投影时的结构过滤，不改变存储不变式。
/// 新增/更新/整表换变量共用这一条规则。
pub(super) fn validate_provider(
    slug: &str,
    provider: &Provider,
    variables: &Variables,
) -> Result<(), StoreError> {
    provider
        .interpolate(variables)
        .map_err(|error| interpolation_error(slug, error))?;
    Ok(())
}
