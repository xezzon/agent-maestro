//! 测试共享助手：临时主目录 + 替身 fetcher 的插件服务与配置存储。

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex, RwLock},
};

pub(super) use super::fetch::testutil::{LockProbeFetcher, StubFetcher};
pub(super) use super::manifest::testutil::manifest_json;
use super::{
    PluginService,
    fetch::Fetcher,
    ports::{PluginEntries, PluginStore, ProjectionSnapshot, ProviderSource},
};
use crate::{
    config::{Config, PluginEntry},
    errors::{ConfigError, PluginEntryError},
    paths::Paths,
    provider::Provider,
    store::{AppStore, Store},
    testutil::setup_paths,
    variables::Variables,
};

/// 不联网的插件服务：任何未预置的拉取都会失败。
pub(crate) fn test_service(home: &Path) -> PluginService {
    stub_service(home, StubFetcher::new())
}

pub(super) fn stub_service(home: &Path, fetcher: StubFetcher) -> PluginService {
    fetcher_service(home, Arc::new(fetcher))
}

/// 注入 fetcher 的插件服务：同时注入全局路径（主目录 + 平台基准目录），
/// 测试服务一律不读真实环境。
pub(super) fn fetcher_service(home: &Path, fetcher: Arc<dyn Fetcher>) -> PluginService {
    setup_paths(home);
    PluginService::with_fetcher(Paths::from_home(home), fetcher)
}

pub(crate) fn store_at(home: &Path) -> AppStore {
    setup_paths(home);
    AppStore {
        store: RwLock::new(Store::new(Paths::from_home(home).config_path())),
    }
}

/// 测试读取配置快照：走与服务同一套短锁访问。
pub(super) fn snapshot(store: &AppStore) -> Config {
    store.with_read(|s| s.config_snapshot()).unwrap()
}

/// 内存配置存储：插件条目、Provider 与变量表全在内存里，不落任何文件。
///
/// 服务层经端口访问配置存储（见 [`crate::plugin::ports`]），故测试可以拿这份
/// 替身跑通装卸与投影——不必先造一个真的配置文件（#141 I-1）。
///
/// 规则与真存储同源：来源即条目唯一身份，id 是来源到落位目录的唯一映射
/// （见 ADR 0006）——重复来源与 id 冲突照旧报错。
pub(crate) struct MemoryStore {
    plugins: Mutex<Vec<PluginEntry>>,
    providers: BTreeMap<String, Provider>,
    variables: Variables,
}

impl MemoryStore {
    /// 空的内存存储：没有条目、没有 Provider、没有变量。
    pub(crate) fn new() -> Self {
        Self {
            plugins: Mutex::new(Vec::new()),
            providers: BTreeMap::new(),
            variables: Variables::new(),
        }
    }

    /// 带上 Provider 表：键为 slug。
    pub(crate) fn with_providers(mut self, providers: BTreeMap<String, Provider>) -> Self {
        self.providers = providers;
        self
    }

    /// 带上全局变量表。
    pub(crate) fn with_variables(mut self, variables: Variables) -> Self {
        self.variables = variables;
        self
    }
}

impl ProviderSource for MemoryStore {
    fn projection_snapshot(&self) -> Result<ProjectionSnapshot, ConfigError> {
        // 端口的契约：投影输入只含启用的 Provider（禁用项不参与，见 #94）——
        // 替身实现的是这条契约，不是存储层的筛选函数。
        let providers = self
            .providers
            .iter()
            .filter(|(_, provider)| provider.enabled)
            .map(|(slug, provider)| (slug.clone(), provider.clone()))
            .collect();
        Ok(ProjectionSnapshot {
            providers,
            variables: self.variables.clone(),
        })
    }
}

impl PluginEntries for MemoryStore {
    fn list_plugins(&self) -> Result<Vec<PluginEntry>, ConfigError> {
        Ok(self.plugins.lock().unwrap().clone())
    }

    fn plugin_by_source(&self, source: &str) -> Result<PluginEntry, PluginEntryError> {
        self.plugins
            .lock()
            .unwrap()
            .iter()
            .find(|entry| entry.source == source)
            .cloned()
            .ok_or_else(|| PluginEntryError::missing_source(source))
    }

    fn add_plugin(&self, entry: &PluginEntry) -> Result<(), PluginEntryError> {
        let mut plugins = self.plugins.lock().unwrap();
        if plugins.iter().any(|other| other.source == entry.source) {
            return Err(PluginEntryError::duplicate_source(&entry.source));
        }
        if let Some(other) = id_conflict(&plugins, &entry.source, &entry.id) {
            return Err(PluginEntryError::duplicate_id(&entry.id, &other.source));
        }
        plugins.push(entry.clone());
        Ok(())
    }

    fn delete_plugin(&self, source: &str) -> Result<Option<PluginEntry>, PluginEntryError> {
        let mut plugins = self.plugins.lock().unwrap();
        // 条目不存在即 `Ok(None)`：与端口契约和 `AppStore` 的适配一致（幂等）。
        Ok(plugins
            .iter()
            .position(|entry| entry.source == source)
            .map(|index| plugins.remove(index)))
    }

    fn set_plugin_enabled(
        &self,
        source: &str,
        enabled: bool,
    ) -> Result<Option<PluginEntry>, PluginEntryError> {
        let mut plugins = self.plugins.lock().unwrap();
        let entry = plugins
            .iter_mut()
            .find(|entry| entry.source == source)
            .ok_or_else(|| PluginEntryError::missing_source(source))?;
        if entry.enabled == enabled {
            return Ok(None);
        }
        let previous = entry.clone();
        entry.enabled = enabled;
        Ok(Some(previous))
    }

    fn check_plugin_id_conflict(&self, source: &str, id: &str) -> Result<(), PluginEntryError> {
        let plugins = self.plugins.lock().unwrap();
        match id_conflict(&plugins, source, id) {
            Some(other) => Err(PluginEntryError::duplicate_id(id, &other.source)),
            None => Ok(()),
        }
    }
}

impl PluginStore for MemoryStore {
    fn plugin_entry_with_variables(
        &self,
        source: &str,
    ) -> Result<(PluginEntry, Variables), PluginEntryError> {
        Ok((self.plugin_by_source(source)?, self.variables.clone()))
    }
}

/// 同一 id 是否已被**其它**来源的条目持有（与真存储同一条规则，见 ADR 0006）：
/// id 是来源到落位目录的唯一映射。
fn id_conflict<'a>(plugins: &'a [PluginEntry], source: &str, id: &str) -> Option<&'a PluginEntry> {
    plugins
        .iter()
        .find(|other| other.source != source && other.id == id)
}
