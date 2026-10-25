//! 全局变量表的错误：名合法性，以及换表后对存量 Provider 的插值复检。

use std::fmt::{self, Display};

use super::{config::ConfigError, impl_storage_error, interpolation::ProviderInterpolationError};

/// 全局变量表的错误：名合法性，以及换表后对存量 Provider 的插值复检。
#[derive(Debug, Clone)]
pub(crate) enum VariableError {
    /// 变量名不合法：占位符无法引用（变量名语法见 ADR 0014）。
    InvalidVariableName { name: String },
    /// 换表后某个 Provider 的插值失败（删除在用变量即整表拒绝）。
    Interpolation(ProviderInterpolationError),
    /// 配置的读取或落盘失败。
    Storage(ConfigError),
}

impl VariableError {
    pub(crate) fn invalid_variable_name(name: &str) -> Self {
        Self::InvalidVariableName {
            name: name.to_owned(),
        }
    }
}

impl_storage_error!(VariableError);

impl Display for VariableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VariableError::InvalidVariableName { name } => write!(
                f,
                "变量名不合法：「{name}」\n变量名只能包含字母、数字与下划线（见 ADR 0014）。"
            ),
            VariableError::Interpolation(error) => write!(f, "{error}"),
            VariableError::Storage(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for VariableError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_variable_name_error_message_is_verbatim() {
        let err = VariableError::InvalidVariableName {
            name: "a-b".to_owned(),
        };

        assert_eq!(
            err.to_string(),
            "变量名不合法：「a-b」\n变量名只能包含字母、数字与下划线（见 ADR 0014）。"
        );
    }
}
