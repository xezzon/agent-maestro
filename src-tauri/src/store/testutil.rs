//! 配置存储的测试夹具：临时主目录 + 就绪的存储。
//!
//! 存储层的每个用例都要「临时主目录 + 已初始化的全局路径」——此前 45 处各写
//! 一遍 `tempfile::tempdir()` + 路径注入，setup 的重复盖过了用例本身。夹具
//! 收在 crate 级 [`crate::testutil`]，存储层与插件层共用同一份形状。

pub(crate) use crate::testutil::temp_home;
