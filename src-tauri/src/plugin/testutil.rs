//! 测试共享助手：临时主目录 + 替身 fetcher 的插件服务与配置存储。

use std::{
    path::Path,
    sync::{Arc, RwLock},
};

pub(super) use super::fetch::testutil::{LockProbeFetcher, StubFetcher};
pub(super) use super::manifest::testutil::manifest_json;
use super::{PluginService, fetch::Fetcher};
use crate::store::{AppStore, Config, Store};

pub(crate) use crate::testutil::{setup_paths, temp_home};

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
    PluginService::with_fetcher(fetcher)
}

pub(crate) fn store_at(home: &Path) -> AppStore {
    setup_paths(home);
    AppStore {
        store: RwLock::new(Store::new()),
    }
}

/// 测试读取配置快照：走与服务同一套短锁访问。
pub(super) fn snapshot(store: &AppStore) -> Config {
    store.with_read(|s| s.config_snapshot()).unwrap()
}
