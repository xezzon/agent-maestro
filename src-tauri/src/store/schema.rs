//! 配置存储的形状：顶层文档、插件条目、存储错误与变更标记。
//!
//! 本模块只声明「配置长什么样」与「出错时怎么说」，不含读写行为——行为分给
//! [`providers`]（Provider 仓储）、[`plugins`]（插件条目仓储）、
//! [`variable_table`]（变量表）与 [`persist`]（原子落盘）。

use std::{collections::BTreeMap, path::PathBuf};

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
/// 条目住在存储层：它是配置文件的形状，不是插件服务的内部状态。插件服务消费
/// 条目，不定义条目——否则存储层与插件服务会互相依赖成环。
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

/// 配置存储的错误；结构化携带原因，面向用户的句式由 `crate::errors` 统一排版。
#[derive(Debug, Clone)]
pub(crate) enum StoreError {
    /// 配置文件存在但无法解析。
    Corrupt { path: PathBuf, detail: String },
    /// 配置文件版本不被当前应用支持。
    UnsupportedVersion { path: PathBuf, found: u32 },
    /// 读写配置文件时发生 IO 错误。
    Io { path: PathBuf, detail: String },
    /// 配置读得出来，却写不出合法的 JSON（序列化失败）。
    ///
    /// 与 IO 失败是两件事：文件没坏、也不是写不动，而是内容写不成 JSON——
    /// 混进 `Io` 会让报错指向错误的修复方向（去查磁盘权限，而不是查内容）。
    Serialize { detail: String },
    /// 已存在同名 Provider。
    DuplicateSlug { slug: String },
    /// Provider 不存在（update/delete 不做 upsert，绝不静默覆盖）。
    MissingSlug { slug: String },
    /// 插件条目不存在。
    MissingSource { source: String },
    /// 已存在同一来源的插件条目（来源即条目唯一身份）。
    DuplicateSource { source: String },
    /// 同一 id 已被其它来源的插件条目持有（id 是来源到落位目录的唯一映射）。
    DuplicateId { id: String, source: String },
    /// Provider 对（变更后的）全局变量表插值失败：变量未定义或占位符语法
    /// 错误。与投影时的插值失败同因同报，只是提前到落盘前拦截
    /// （provider↔variables 一致性，ADR 0015）。句式由 `crate::errors` 一处
    /// 产出，与投影时逐字一致。
    Interpolation {
        slug: String,
        field: String,
        reason: String,
    },
    /// 变量名不合法：占位符无法引用（变量名语法见 ADR 0014）。
    InvalidVariableName { name: String },
    /// 配置存储的锁不可用（其它命令持锁期间 panic 毒化了锁）。
    ///
    /// 由 [`crate::store::LockPoisoned`] 折入：毒化的是 `AppStore` 的锁而非配置
    /// 本身，但两条加锁路径对用户是同一件事（「配置存储不可用」，见
    /// [`crate::store::STORE_LOCK_POISONED`]）。
    Unavailable,
}

impl StoreError {
    /// 「Provider 不存在」错误：update/delete 不做 upsert，绝不静默覆盖。
    pub(super) fn missing_slug(slug: &str) -> Self {
        Self::MissingSlug {
            slug: slug.to_owned(),
        }
    }

    /// 「已存在同名 Provider」错误。
    pub(super) fn duplicate_slug(slug: &str) -> Self {
        Self::DuplicateSlug {
            slug: slug.to_owned(),
        }
    }

    /// 「插件条目不存在」错误：按来源定位不到条目即报错，绝不静默写入。
    pub(super) fn missing_source(source: &str) -> Self {
        Self::MissingSource {
            source: source.to_owned(),
        }
    }

    /// 「已存在同一来源的插件条目」错误。
    pub(super) fn duplicate_source(source: &str) -> Self {
        Self::DuplicateSource {
            source: source.to_owned(),
        }
    }

    /// 「同一 id 已被其它来源的插件条目持有」错误。
    pub(super) fn duplicate_id(id: &str, source: &str) -> Self {
        Self::DuplicateId {
            id: id.to_owned(),
            source: source.to_owned(),
        }
    }

    /// 「变量名不合法」错误：占位符无法引用。
    pub(super) fn invalid_variable_name(name: &str) -> Self {
        Self::InvalidVariableName {
            name: name.to_owned(),
        }
    }
}

/// 一次配置变更的结果：要不要落盘，以及要带回给调用方的领域值。
///
/// `Changed` 落盘并替换内存状态；`Unchanged` 什么都不做（幂等路径：
/// 状态未变即不写盘）。两者都带回领域值——「没变化」不等于「没有返回值」。
pub(super) enum Change<T> {
    /// 有变化：落盘成功后替换内存状态。
    Changed(T),
    /// 无变化：不写盘、不动内存状态。
    Unchanged(T),
}
