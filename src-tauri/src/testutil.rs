//! 测试夹具：临时主目录 + 已就位的全局路径。
//!
//! 存储层与插件层此前各有一份 `temp_home()`——形状相同、注入的全局却不同：
//! 存储层只注入主目录，插件层还要注入平台基准目录，于是同一个临时主目录下
//! 两层看到的世界并不一致。合成一处后，两个模块的用例拿到同一份目录布局：
//! 插件的 `config_dir` 解析（ADR 0011）落在同一个临时主目录之下。

use std::path::Path;

use crate::paths::Paths;

/// 测试用临时主目录：建好目录并注入全局路径（主目录 + 平台基准目录）。
///
/// 目录由返回值持有，测试期间不会被提前删掉——调用方把它绑成 `_home`
/// （不用的绑定加下划线前缀，免得编译器报「未使用的变量」）。
pub(crate) fn temp_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    setup_paths(home.path());
    home
}

/// 注入全局路径（主目录 + 平台基准目录）。
///
/// 全局槽在测试期是线程局部的（见 [`crate::test_global`]）：跨线程的用例要在
/// 子线程里重新注入，否则子线程看到的是空的全局。
pub(crate) fn setup_paths(home: &Path) {
    Paths::init_test(Paths::from_home(home));
}
