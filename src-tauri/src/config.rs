//! 配置文档的形状：顶层文档、插件条目与变更标记。
//!
//! 本模块只声明「配置长什么样」，不含读写行为——行为分给 [`crate::store`]
//! （Provider 仓储、插件条目仓储、变量表、原子落盘与保护态），错误类型住在
//! crate 根的 [`crate::errors`]。
//!
//! 住在 crate 根而非 `store/`：这些词汇被存储层与插件服务**两侧**消费（插件
//! 服务读写插件条目，存储层把它们落盘）。若词汇住在存储层，插件服务就得依赖
//! 存储层；而存储层又要实现插件服务声明的端口（见 [`crate::plugin::ports`]）——
//! 两侧会因此互相依赖成环。放在中立层，两边各自单向依赖它（#141 I-1）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{provider::Provider, variables::Variables};

/// 配置文件 schema 版本（见 ADR 0001）。
pub(crate) const CONFIG_VERSION: u32 = 1;

/// `~/.maestro/config.json` 的顶层文档（version 1 schema）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Config {
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) providers: BTreeMap<String, Provider>,
    /// 全局变量表（ADR 0015）：名 → 默认值。纯增量字段：缺键的旧文件照常读入得空表。
    #[serde(default)]
    pub(crate) variables: Variables,
    /// 无插件时省略该段，保持与旧配置文件一致。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) plugins: Vec<PluginEntry>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            providers: BTreeMap::new(),
            variables: Variables::new(),
            plugins: Vec::new(),
        }
    }
}

/// 插件条目（config.json 的 `plugins` 段，纯增量字段；见 issue #34）。
///
/// `source` 是条目唯一身份（内置 `builtin:<id>`、指向 manifest.json 的 https URL
/// 或本机绝对路径），重复添加在 store 层拒绝；`id` 为插件 id，条目在安装成功后
/// 写入（内置插件由启动时装载管线补写，与其余来源同一流程）——它是来源到落位
/// 目录的唯一映射（见 ADR 0006）。
///
/// 条目住在中立层：它是配置文件的形状，不是任何一侧的内部状态。存储层把它
/// 落盘，插件服务消费条目——两侧都不定义它，故不互相依赖（#141 I-1）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PluginEntry {
    pub(crate) source: String,
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) id: String,
}

fn default_true() -> bool {
    true
}

/// 一次配置变更的结果：要不要落盘，以及要带回给调用方的领域值。
///
/// `Changed` 落盘并替换内存状态；`Unchanged` 什么都不做（幂等路径：
/// 状态未变即不写盘）。两者都带回领域值——「没变化」不等于「没有返回值」。
pub(crate) enum Change<T> {
    /// 有变化：落盘成功后替换内存状态。
    Changed(T),
    /// 无变化：不写盘、不动内存状态。
    Unchanged(T),
}
