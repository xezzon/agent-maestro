//! 配置存储的端口实现：插件服务声明的能力（见 [`crate::plugin::ports`]）在这里
//! 落到 `AppStore` 上。
//!
//! 这一层是适配器：它把插件服务要的窄能力接到 `AppStore` 的读写锁上，每个
//! 操作收敛成一把锁内的短临界区——插件服务拿不到锁，也就不会在锁内做 IO。
//!
//! 端口 trait 由服务层声明、由本层实现，故本层单向依赖 [`crate::plugin`]：
//! 依赖箭头指向内层，模块图不成环（#141 I-1）。

use crate::{
    config::PluginEntry,
    errors::{ConfigError, PluginEntryError},
    plugin::ports::{PluginEntries, ProjectionSnapshot, ProviderSource},
    variables::Variables,
};

use super::AppStore;

impl ProviderSource for AppStore {
    fn projection_snapshot(&self) -> Result<ProjectionSnapshot, ConfigError> {
        // 一次读锁内取两份快照：投影按同一份配置插值（见 `ProjectionSnapshot`）。
        self.with_read(|store| {
            Ok(ProjectionSnapshot {
                providers: store.enabled_providers()?,
                variables: store.variables_snapshot()?,
            })
        })
    }

    fn variables_snapshot(&self) -> Result<Variables, ConfigError> {
        self.with_read(|store| store.variables_snapshot())
    }
}

impl PluginEntries for AppStore {
    fn list_plugins(&self) -> Result<Vec<PluginEntry>, ConfigError> {
        self.with_read(|store| store.list_plugins())
    }

    fn plugin_by_source(&self, source: &str) -> Result<PluginEntry, PluginEntryError> {
        self.with_read(|store| store.plugin_by_source(source))
    }

    fn add_plugin(&self, entry: &PluginEntry) -> Result<(), PluginEntryError> {
        self.with_write(|store| store.add_plugin(entry))
    }

    fn delete_plugin(&self, source: &str) -> Result<Option<PluginEntry>, PluginEntryError> {
        // 找条目 + 删条目同处一把写锁：两步之间不留「条目已找、条目未删」的窗口。
        self.with_write(|store| match store.delete_plugin(source) {
            Ok(entry) => Ok(Some(entry)),
            // 条目已不存在：幂等（调用方据此认定「已移除」）。
            Err(PluginEntryError::MissingSource { .. }) => Ok(None),
            Err(err) => Err(err),
        })
    }

    fn set_plugin_enabled(
        &self,
        source: &str,
        enabled: bool,
    ) -> Result<Option<PluginEntry>, PluginEntryError> {
        // 读-判断-写同处一把写锁：并发方已完成切换时返回 None（不写盘）。
        self.with_write(|store| {
            let entry = store.plugin_by_source(source)?;
            if entry.enabled == enabled {
                return Ok(None);
            }
            store.set_plugin_enabled(source, enabled)?;
            Ok(Some(entry))
        })
    }

    fn check_plugin_id_conflict(&self, source: &str, id: &str) -> Result<(), PluginEntryError> {
        self.with_read(|store| store.check_plugin_id_conflict(source, id))
    }
}
