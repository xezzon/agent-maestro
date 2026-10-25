//! 配置文档的形状：顶层文档、插件条目、存储错误与变更标记。
//!
//! 本模块只声明「配置长什么样」与「出错时怎么说」，不含读写行为——行为分给
//! [`crate::store`]（Provider 仓储、插件条目仓储、变量表、原子落盘与保护态）。
//!
//! 住在 crate 根而非 `store/`：这些词汇被存储层与插件服务**两侧**消费（插件
//! 服务读写插件条目，存储层把它们落盘）。若词汇住在存储层，插件服务就得依赖
//! 存储层；而存储层又要实现插件服务声明的端口（见 [`crate::plugin::ports`]）——
//! 两侧会因此互相依赖成环。放在中立层，两边各自单向依赖它（#141 I-1）。

use std::{collections::BTreeMap, fmt::Display, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    provider::{Provider, ProviderInterpolationError},
    variables::Variables,
};

/// 配置文件 schema 版本（见 ADR 0001）。
pub(crate) const CONFIG_VERSION: u32 = 1;

/// 配置存储的锁被毒化（其它命令持锁期间 panic）时对用户可见的原因。
///
/// 住在中立层而非存储层：说这句文案的是 [`StoreError`]（命令层经它排版），
/// 若文案跟着锁一起住在存储层，中立层就要反向依赖存储层——依赖箭头会成环。
pub(crate) const STORE_LOCK_POISONED: &str = "配置存储不可用";

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

/// 配置存储的错误；结构化携带原因，面向用户的句式由本类型的 [`Display`]
/// 一处产出（消费方各自把它格式化成字符串——IPC 错误通道只收字符串）。
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
    /// （provider↔variables 一致性，ADR 0015）。包裹领域层的同一个错误类型，
    /// 句式由它排版（见 [`ProviderInterpolationError`]），两条路径逐字一致。
    Interpolation(ProviderInterpolationError),
    /// 变量名不合法：占位符无法引用（变量名语法见 ADR 0014）。
    InvalidVariableName { name: String },
    /// 配置存储的锁不可用（其它命令持锁期间 panic 毒化了锁）。
    ///
    /// 由 [`crate::store::LockPoisoned`] 折入：毒化的是 `AppStore` 的锁而非配置
    /// 本身，但两条加锁路径对用户是同一件事（「配置存储不可用」，见
    /// [`STORE_LOCK_POISONED`]）。
    Unavailable,
}

impl StoreError {
    /// 「Provider 不存在」错误：update/delete 不做 upsert，绝不静默覆盖。
    pub(crate) fn missing_slug(slug: &str) -> Self {
        Self::MissingSlug {
            slug: slug.to_owned(),
        }
    }

    /// 「已存在同名 Provider」错误。
    pub(crate) fn duplicate_slug(slug: &str) -> Self {
        Self::DuplicateSlug {
            slug: slug.to_owned(),
        }
    }

    /// 「插件条目不存在」错误：按来源定位不到条目即报错，绝不静默写入。
    pub(crate) fn missing_source(source: &str) -> Self {
        Self::MissingSource {
            source: source.to_owned(),
        }
    }

    /// 「已存在同一来源的插件条目」错误。
    pub(crate) fn duplicate_source(source: &str) -> Self {
        Self::DuplicateSource {
            source: source.to_owned(),
        }
    }

    /// 「同一 id 已被其它来源的插件条目持有」错误。
    pub(crate) fn duplicate_id(id: &str, source: &str) -> Self {
        Self::DuplicateId {
            id: id.to_owned(),
            source: source.to_owned(),
        }
    }

    /// 「变量名不合法」错误：占位符无法引用。
    pub(crate) fn invalid_variable_name(name: &str) -> Self {
        Self::InvalidVariableName {
            name: name.to_owned(),
        }
    }
}

impl Display for StoreError {
    /// 面向最终用户的一句话：把结构化原因排版成可读文案。
    ///
    /// 句式由本实现一处产出——命令层与插件服务各自把本错误格式化成字符串
    /// （IPC 错误通道只收字符串），同因同报、逐字一致。插值失败一句转给
    /// 领域层的 [`ProviderInterpolationError`]（同一句式只有那一份实现）。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Corrupt { path, detail } => write!(
                f,
                "配置文件已损坏：{}\n原因：{detail}\n请修复或删除该文件后重启应用；在此之前 Maestro 拒绝任何写入，绝不会静默重建。",
                path.display()
            ),
            StoreError::UnsupportedVersion { path, found } => write!(
                f,
                "配置文件版本不受支持：{}\n文件中的 version 为 {found}，当前应用仅支持 {CONFIG_VERSION}。\n请修复该文件后重启应用；在此之前 Maestro 拒绝任何写入，绝不会静默重建。",
                path.display()
            ),
            StoreError::Io { path, detail } => {
                write!(f, "读写配置文件失败：{}\n原因：{detail}", path.display())
            }
            StoreError::Serialize { detail } => {
                write!(f, "序列化配置文件失败\n原因：{detail}")
            }
            StoreError::DuplicateSlug { slug } => write!(f, "已存在同名 Provider：{slug}"),
            StoreError::MissingSlug { slug } => write!(f, "Provider 不存在：{slug}"),
            StoreError::MissingSource { source } => write!(f, "插件条目不存在：{source}"),
            StoreError::DuplicateSource { source } => {
                write!(f, "已存在同一来源的插件条目：{source}")
            }
            StoreError::DuplicateId { id, source } => {
                write!(f, "插件 id「{id}」已被来源 {source} 占用")
            }
            StoreError::Interpolation(error) => write!(f, "{error}"),
            StoreError::InvalidVariableName { name } => write!(
                f,
                "变量名不合法：「{name}」\n变量名只能包含字母、数字与下划线（见 ADR 0014）。"
            ),
            StoreError::Unavailable => write!(f, "{}", STORE_LOCK_POISONED),
        }
    }
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::provider::FieldError;

    /// 落盘前拦截的插值失败文案逐字断言：slug、字段名、原因三者都在句子里，
    /// 且与投影时拦截同一句式（ADR 0015「报错与投影时同因同报」；句式由
    /// [`ProviderInterpolationError`] 独家产出）。
    #[test]
    fn interpolation_store_error_message_is_verbatim() {
        let err = StoreError::Interpolation(ProviderInterpolationError {
            slug: "gateway".to_owned(),
            error: FieldError {
                field: "base_url.openai-completions".to_owned(),
                reason: "No such variable: $HOST".to_owned(),
            },
        });

        assert_eq!(
            err.to_string(),
            "Provider「gateway」的 base_url.openai-completions 插值失败：No such variable: $HOST"
        );
    }

    /// 保护态文案逐字钉住：路径、原因与修复指引三段排版都在。
    #[test]
    fn corrupt_error_message_is_verbatim() {
        let err = StoreError::Corrupt {
            path: PathBuf::from("/home/u/.maestro/config.json"),
            detail: "expected value at line 1 column 1".to_owned(),
        };

        assert_eq!(
            err.to_string(),
            "配置文件已损坏：/home/u/.maestro/config.json\n原因：expected value at line 1 column 1\n请修复或删除该文件后重启应用；在此之前 Maestro 拒绝任何写入，绝不会静默重建。"
        );
    }

    #[test]
    fn unsupported_version_error_message_is_verbatim() {
        let err = StoreError::UnsupportedVersion {
            path: PathBuf::from("/home/u/.maestro/config.json"),
            found: 99,
        };

        assert_eq!(
            err.to_string(),
            "配置文件版本不受支持：/home/u/.maestro/config.json\n文件中的 version 为 99，当前应用仅支持 1。\n请修复该文件后重启应用；在此之前 Maestro 拒绝任何写入，绝不会静默重建。"
        );
    }

    #[test]
    fn io_error_message_is_verbatim() {
        let err = StoreError::Io {
            path: PathBuf::from("/home/u/.maestro/config.json"),
            detail: "Permission denied (os error 13)".to_owned(),
        };

        assert_eq!(
            err.to_string(),
            "读写配置文件失败：/home/u/.maestro/config.json\n原因：Permission denied (os error 13)"
        );
    }

    #[test]
    fn serialize_error_message_is_verbatim() {
        let err = StoreError::Serialize {
            detail: "serializer error".to_owned(),
        };

        assert_eq!(
            err.to_string(),
            "序列化配置文件失败\n原因：serializer error"
        );
    }

    #[test]
    fn slug_and_source_error_messages_are_verbatim() {
        assert_eq!(
            StoreError::DuplicateSlug {
                slug: "foo".to_owned()
            }
            .to_string(),
            "已存在同名 Provider：foo"
        );
        assert_eq!(
            StoreError::MissingSlug {
                slug: "ghost".to_owned()
            }
            .to_string(),
            "Provider 不存在：ghost"
        );
        assert_eq!(
            StoreError::MissingSource {
                source: "builtin:pi".to_owned()
            }
            .to_string(),
            "插件条目不存在：builtin:pi"
        );
        assert_eq!(
            StoreError::DuplicateSource {
                source: "https://example.com/manifest.json".to_owned()
            }
            .to_string(),
            "已存在同一来源的插件条目：https://example.com/manifest.json"
        );
        assert_eq!(
            StoreError::DuplicateId {
                id: "pi".to_owned(),
                source: "builtin:pi".to_owned()
            }
            .to_string(),
            "插件 id「pi」已被来源 builtin:pi 占用"
        );
    }

    #[test]
    fn invalid_variable_name_error_message_is_verbatim() {
        let err = StoreError::InvalidVariableName {
            name: "a-b".to_owned(),
        };

        assert_eq!(
            err.to_string(),
            "变量名不合法：「a-b」\n变量名只能包含字母、数字与下划线（见 ADR 0014）。"
        );
    }
}
