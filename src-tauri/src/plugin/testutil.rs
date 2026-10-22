//! 测试共享助手：临时主目录 + 替身 fetcher 的插件服务与配置存储。

use std::{
    path::Path,
    sync::{Arc, RwLock},
};

pub(super) use super::fetch::testutil::{LockProbeFetcher, StubFetcher};
pub(super) use super::manifest::testutil::manifest_json;
use super::{PluginService, fetch::Fetcher};
use crate::{
    paths::MaestroPaths,
    plugin::manifest::PlatformDirs,
    store::{AppStore, Config, Store},
};

pub(crate) fn temp_home() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// 设置测试用全局目录（MaestroPaths + PlatformDirs），落在临时主目录下。
pub(super) fn setup_test_dirs(home: &Path) {
    MaestroPaths::init(home);
    PlatformDirs::init_test(PlatformDirs::new(
        home.to_path_buf(),
        home.join(".config"),
        home.join(".config"),
        home.join(".local/share"),
        home.join(".local/share"),
    ));
}

/// 不联网的插件服务：任何未预置的拉取都会失败。
pub(crate) fn test_service(home: &Path) -> PluginService {
    stub_service(home, StubFetcher::new())
}

pub(super) fn stub_service(home: &Path, fetcher: StubFetcher) -> PluginService {
    fetcher_service(home, Arc::new(fetcher))
}

/// 注入 fetcher 的插件服务：同时设置全局 MaestroPaths 与 PlatformDirs，
/// 测试服务一律不读真实环境。
pub(super) fn fetcher_service(home: &Path, fetcher: Arc<dyn Fetcher>) -> PluginService {
    setup_test_dirs(home);
    PluginService::with_fetcher(fetcher)
}

pub(crate) fn store_at(home: &Path) -> AppStore {
    MaestroPaths::init(home);
    AppStore {
        store: RwLock::new(Store::new()),
    }
}

/// 测试读取配置快照：走与服务同一套短锁访问。
pub(super) fn snapshot(store: &AppStore) -> Config {
    store.read().unwrap().get().unwrap().clone()
}
