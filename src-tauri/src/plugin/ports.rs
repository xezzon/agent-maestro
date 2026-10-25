//! 插件服务对配置存储的端口：服务层只声明它真正需要的能力。
//!
//! 端口住在服务层（这里）：插件服务因此不碰配置存储的具体类型——既拿不到
//! 它的读写锁，也拿不到它的落盘时机。「不得持锁做 IO」这条纪律不再靠注释
//! 提醒，而是类型上根本没有让它持锁的入口（#141 I-1）。
//!
//! 实现在 [`crate::store::ports`]：那边把能力落到配置存储上，并把每个操作
//! 收敛成一把锁内的短临界区。

use std::collections::BTreeMap;

use crate::{
    config::{PluginEntry, StoreError},
    provider::Provider,
    variables::Variables,
};

/// 投影输入的快照：启用的 Provider 表 ⊕ 全局变量表。
///
/// 两者取自**同一次**读锁：投影按同一份配置插值，两次读之间配置若被改，
/// 插值就会拿旧 Provider 配新变量——投影整体失败，且失败原因与真实配置
/// 对不上。
pub(crate) struct ProjectionSnapshot {
    /// 参与投影的 Provider 表（禁用项已被剔除，见 #94）。
    pub(crate) providers: BTreeMap<String, Provider>,
    /// 全局变量表（ADR 0015）。
    pub(crate) variables: Variables,
}

/// 只读端口：投影要什么就声明什么。
///
/// 投影路径只读 Provider 表与全局变量表，不写条目——它因此只需要这一条
/// 只读能力，测试里换一份内存替身即可（不落任何文件）。
pub(crate) trait ProviderSource {
    /// 投影输入的快照（Provider 表与变量表同源）。
    fn projection_snapshot(&self) -> Result<ProjectionSnapshot, StoreError>;
    /// 全局变量表：写插件配置时校验覆盖项用（不必连带 Provider 表）。
    fn variables_snapshot(&self) -> Result<Variables, StoreError>;
}

/// 插件条目端口：条目是配置文件的形状，服务层按来源读写它们。
pub(crate) trait PluginEntries {
    /// 全部插件条目。
    fn list_plugins(&self) -> Result<Vec<PluginEntry>, StoreError>;
    /// 按来源取条目；不存在即报错（来源即条目唯一身份）。
    fn plugin_by_source(&self, source: &str) -> Result<PluginEntry, StoreError>;
    /// 新增条目（来源重复或 id 冲突即报错）。
    fn add_plugin(&self, entry: &PluginEntry) -> Result<(), StoreError>;
    /// 删除条目，返回被删的那一条。
    fn delete_plugin(&self, source: &str) -> Result<PluginEntry, StoreError>;
    /// 切换启用态：条目不存在即报错；已处于目标态则不写盘并返回 `None`。
    ///
    /// 读-判断-写在同一把锁内完成（适配器侧的纪律）：并发方已完成切换时
    /// 返回 `None`，调用方据此认定「状态已是目标态」。
    fn set_plugin_enabled(
        &self,
        source: &str,
        enabled: bool,
    ) -> Result<Option<PluginEntry>, StoreError>;
    /// id 冲突预检：同一 id 不得被**其它**来源的条目持有。
    fn check_plugin_id_conflict(&self, source: &str, id: &str) -> Result<(), StoreError>;
}

/// 组合端口：既要条目又要变量表的入口（如写插件配置）收它。
///
/// 两条能力各自独立成 trait 是为了「窄」：投影只拿 [`ProviderSource`]，
/// 装卸只拿 [`PluginEntries`]；这个组合 trait 只是让「两条都要」的入口不必
/// 收两个参数。
pub(crate) trait PluginStore: ProviderSource + PluginEntries {}

impl<T: ProviderSource + PluginEntries> PluginStore for T {}
