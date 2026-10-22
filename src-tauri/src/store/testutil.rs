//! 配置存储的测试夹具：临时主目录 + 就绪的存储。
//!
//! 存储层的每个用例都要「临时主目录 + 已初始化的全局主目录」——此前 45 处
//! 各写一遍 `tempfile::tempdir()` + `MaestroPaths::init()`，setup 的重复盖过
//! 了用例本身。形状照 `plugin::testutil`（已证明可用）。

use crate::paths::MaestroPaths;

/// 测试用临时主目录：建好目录并初始化全局主目录。
///
/// 目录由返回值持有，测试期间不会被提前删掉——调用方把它绑成 `_home`
/// （不用的绑定加下划线前缀，免得编译器报「未使用的变量」）。
pub(crate) fn temp_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    MaestroPaths::init(home.path());
    home
}
