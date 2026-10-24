use std::collections::BTreeMap;

use tauri::State;

use super::log_outcome;
use crate::{errors::user_message, store::AppStore};

#[tauri::command]
pub(crate) fn list_variables(
    store: State<'_, AppStore>,
) -> Result<BTreeMap<String, String>, String> {
    let outcome = store
        .with_read(|s| s.variables_snapshot())
        .map_err(user_message);
    log_outcome("list_variables", None, &outcome);
    outcome
}

/// 整包替换全局变量表（ADR 0015）：新增、修改与删除都由前端整表回传。
/// 变量默认值可能承载敏感信息，识别参数一律不进日志（ADR 0008）。
#[tauri::command]
pub(crate) fn set_variables(
    store: State<'_, AppStore>,
    variables: BTreeMap<String, String>,
) -> Result<(), String> {
    let outcome = store
        .with_write(|s| s.set_variables(variables))
        .map_err(user_message);
    log_outcome("set_variables", None, &outcome);
    outcome
}
