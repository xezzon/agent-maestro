//! 变量表：全局变量（ADR 0015）的整包替换与名合法性。
//!
//! 变量名合法性（ADR 0014）与「改了变量表要对全部 Provider 复检插值」这两条
//! 规则住在这里，存储层与投影层共用一份口径。

use crate::variables::Variables;

use super::{
    providers::validate_provider,
    schema::{Change, Config, StoreError},
};

/// 变量名合法性（ADR 0014）：`[A-Za-z0-9_]+`，与占位符语法一致。
pub(super) fn is_valid_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// 整包替换全局变量表（ADR 0015）：新增、修改与删除都走同一条全量保存路径。
///
/// 变量名须合法（`[A-Za-z0-9_]+`，ADR 0014），否则占位符无法引用；落盘前
/// 用新表对全部存量 Provider（含禁用者）插值一次：删除导致任一 Provider 的
/// 引用无法解析时整表拒绝（provider↔variables 一致性）。空默认值合法
/// （`${EMPTY}`）。
pub(super) fn replace(config: &mut Config, variables: Variables) -> Result<Change<()>, StoreError> {
    // 变量名合法性先于存储状态：非法名整表拒绝，与当前配置是否可读无关。
    for name in variables.keys() {
        if !is_valid_name(name) {
            return Err(StoreError::invalid_variable_name(name));
        }
    }
    for (slug, provider) in &config.providers {
        validate_provider(slug, provider, &variables)?;
    }
    config.variables = variables;
    Ok(Change::Changed(()))
}
