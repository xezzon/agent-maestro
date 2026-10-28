//! 测试夹具：临时主目录 + 内存密钥链 + 配置存储。
//!
//! 密钥链用内存替身（[`MemorySecrets`]）：测试因此能断言「密钥链被写了什么」
//! 「哪个动作压根没被调用」，且真值绝不落盘（ADR 0008）。
//!
//! 夹具是**构造配置存储的唯一入口**：存储层与插件层的测试都从这里拿 Store，
//! 于是「密钥链注入」这件事只有一处做法，不会各自漂移。

use std::{fs, path::Path, sync::Arc};

use crate::{
    paths::Paths,
    secret::{MemorySecrets, SecretStore},
    store::Store,
};

/// 测试夹具：临时主目录 + 内存密钥链 + 配置存储。
pub(crate) struct Fixture {
    /// 临时主目录：`new` 建的夹具持有它（测试期间不会被提前删掉）；
    /// `at` 建的夹具用调用方给的主目录，这里是 `None`。
    _home: Option<tempfile::TempDir>,
    paths: Paths,
    secrets: Arc<MemorySecrets>,
}

impl Fixture {
    /// 新建夹具：空的临时主目录 + 空的内存密钥链。
    pub(crate) fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::from_home(home.path());
        Self {
            _home: Some(home),
            paths,
            secrets: Arc::new(MemorySecrets::new()),
        }
    }

    /// 以给定主目录建夹具：主目录由调用方持有（插件测试与服务共用同一份）。
    pub(crate) fn at(home: &Path) -> Self {
        Self {
            _home: None,
            paths: Paths::from_home(home),
            secrets: Arc::new(MemorySecrets::new()),
        }
    }

    /// 全局路径（配置文件的落位）。
    pub(crate) fn paths(&self) -> &Paths {
        &self.paths
    }

    /// 内存密钥链：断言用。
    pub(crate) fn secrets(&self) -> &Arc<MemorySecrets> {
        &self.secrets
    }

    /// 打开配置存储：读盘，密钥链是同一份内存替身。
    pub(crate) fn store(&self) -> Store {
        let secrets: Arc<dyn SecretStore> = self.secrets.clone();
        Store::new(self.paths.config_path(), secrets)
    }

    /// 写一份配置文件（测试的起点）：目录一并建好。
    pub(crate) fn write_config(&self, text: &str) {
        let path = self.paths.config_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}
