//! 宿主路径解析失败：主目录或平台基准目录不可用。
//!
//! 路径解析失败也是错误，故归错误域。依赖方向：`paths` 依赖 `errors` 是正常的
//! （[`crate::paths::Paths::init_from_system`] 返回本类型），反过来 `errors` 不该
//! 依赖 `paths`——故错误域不认识平台词表，那份清单由调用方填进错误（见
//! [`crate::errors::PluginError::ConfigDirVariableRequired`] 的 `supported` 字段）。

#[derive(Debug, Clone)]
pub(crate) struct PathError {
    what: &'static str,
}

impl PathError {
    #[cfg(not(test))]
    pub(crate) fn unavailable(what: &'static str) -> Self {
        Self { what }
    }
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "无法解析平台目录：{}不可用", self.what)
    }
}

impl std::error::Error for PathError {}
