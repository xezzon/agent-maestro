//! 插件条目仓储：插件条目的新增、删除与启停。
//!
//! 条目是配置文件的形状（见 [`super::schema::PluginEntry`]），规则与 Provider
//! 同源：来源即条目唯一身份，id 是来源到落位目录的唯一映射（见 ADR 0006）。

use super::schema::{Change, Config, PluginEntry, StoreError};

/// 新增插件条目，追加在现有条目之后。
///
/// 安装成功后由插件服务调用，`id` 随条目一并给出：它是来源到落位目录的唯一映射
/// （见 ADR 0006）。因此安装失败时不会留下条目，重新添加即可。
///
/// 同一临界区内复检来源与 id 冲突：插件服务的预检（`check_id_conflict`）与
/// 此处写入分属两次加锁，两次锁之间另一来源可能已占用同一 id。
pub(super) fn add(
    config: &mut Config,
    plugin_entry: &PluginEntry,
) -> Result<Change<()>, StoreError> {
    let source = plugin_entry.source.clone();
    if config.plugins.iter().any(|plugin| plugin.source == source) {
        return Err(StoreError::duplicate_source(&source));
    }
    if let Some(other) = config
        .plugins
        .iter()
        .find(|plugin| plugin.source != source && plugin.id == plugin_entry.id)
    {
        return Err(StoreError::duplicate_id(&plugin_entry.id, &other.source));
    }
    config.plugins.push(plugin_entry.clone());
    Ok(Change::Changed(()))
}

/// 删除插件条目，返回被删除的条目（调用方据此删除落位目录）。
pub(super) fn delete(config: &mut Config, source: &str) -> Result<Change<PluginEntry>, StoreError> {
    let index = config
        .plugins
        .iter()
        .position(|plugin| plugin.source == source)
        .ok_or_else(|| StoreError::missing_source(source))?;
    let removed = config.plugins.remove(index);
    Ok(Change::Changed(removed))
}

/// 启用/禁用插件条目。
pub(super) fn set_enabled(
    config: &mut Config,
    source: &str,
    enabled: bool,
) -> Result<Change<()>, StoreError> {
    let entry = config
        .plugins
        .iter_mut()
        .find(|plugin| plugin.source == source)
        .ok_or_else(|| StoreError::missing_source(source))?;
    entry.enabled = enabled;
    Ok(Change::Changed(()))
}
