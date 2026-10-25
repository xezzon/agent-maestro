//! 插件条目仓储的错误：来源即条目唯一身份，id 是来源到落位目录的唯一映射。

use std::fmt::{self, Display};

use super::{config::ConfigError, impl_storage_error};

/// 插件条目仓储的错误：来源即条目唯一身份，id 是来源到落位目录的唯一映射。
#[derive(Debug, Clone)]
pub(crate) enum PluginEntryError {
    /// 插件条目不存在。
    MissingSource { source: String },
    /// 已存在同一来源的插件条目。
    DuplicateSource { source: String },
    /// 同一 id 已被其它来源的插件条目持有。
    DuplicateId { id: String, source: String },
    /// 配置的读取或落盘失败。
    Storage(ConfigError),
}

impl PluginEntryError {
    pub(crate) fn missing_source(source: &str) -> Self {
        Self::MissingSource {
            source: source.to_owned(),
        }
    }

    pub(crate) fn duplicate_source(source: &str) -> Self {
        Self::DuplicateSource {
            source: source.to_owned(),
        }
    }

    pub(crate) fn duplicate_id(id: &str, source: &str) -> Self {
        Self::DuplicateId {
            id: id.to_owned(),
            source: source.to_owned(),
        }
    }
}

impl_storage_error!(PluginEntryError);

impl Display for PluginEntryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginEntryError::MissingSource { source } => write!(f, "插件条目不存在：{source}"),
            PluginEntryError::DuplicateSource { source } => {
                write!(f, "已存在同一来源的插件条目：{source}")
            }
            PluginEntryError::DuplicateId { id, source } => {
                write!(f, "插件 id「{id}」已被来源 {source} 占用")
            }
            PluginEntryError::Storage(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for PluginEntryError {}
