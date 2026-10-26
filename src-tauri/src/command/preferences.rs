use tauri::State;

use super::log_outcome;
use crate::preferences::{Preferences, PreferencesStore};

/// 读取偏好：返回整份偏好文档。
///
/// 偏好存储在内存中始终有值（读不出来时是兜底后的默认值），故读不到只可能是
/// 锁不可用——报错文案面向用户。
#[tauri::command]
pub(crate) fn get_preferences(store: State<'_, PreferencesStore>) -> Result<Preferences, String> {
    let outcome = store.get().map_err(|err| err.to_string());
    // 识别参数只有主题一项：偏好里没有凭证，落盘内容不进日志。
    let context = outcome
        .as_ref()
        .ok()
        .map(|preferences| format!("theme={}", preferences.theme.as_str()));
    log_outcome("get_preferences", context.as_deref(), &outcome);
    outcome
}

/// 整包替换偏好：后端强制版本号后原子写回磁盘，并更新内存状态。
///
/// 载荷是整份偏好（字段缺省有缺省值）；主题取值非法在反序列化时即被拒——命令
/// 边界严格，磁盘读入宽松（见 ADR 0023）。
#[tauri::command]
pub(crate) fn update_preferences(
    store: State<'_, PreferencesStore>,
    preferences: Preferences,
) -> Result<Preferences, String> {
    let context = format!("theme={}", preferences.theme.as_str());
    let outcome = store.update(preferences).map_err(|err| err.to_string());
    log_outcome("update_preferences", Some(&context), &outcome);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferences::{Loaded, PREFERENCES_VERSION, PreferencesFile, Theme};

    /// 命令边界拒绝未知主题值：反序列化失败，命令根本不进来（写入严格）。
    #[test]
    fn unknown_theme_is_rejected_at_the_command_boundary() {
        assert!(
            serde_json::from_str::<Preferences>(r#"{"version":1,"theme":"neon"}"#).is_err(),
            "未知主题值不得被静默接受"
        );
        assert!(
            serde_json::from_str::<Preferences>(r#"{"version":1,"theme":42}"#).is_err(),
            "非字符串主题值同样被拒"
        );
    }

    /// 磁盘读入则兜底：同一个非法值读进内存时是跟随系统，且整份文档不失败
    /// （读入宽松）——与上面的严格边界恰好相反。
    #[test]
    fn disk_read_falls_back_instead_of_failing() {
        let file: PreferencesFile =
            serde_json::from_str(r#"{"version":1,"theme":"neon"}"#).unwrap();

        match file.normalize() {
            Loaded::Usable {
                preferences,
                needs_rewrite,
            } => {
                assert_eq!(preferences.theme, Theme::System, "非法值兜底为跟随系统");
                assert!(needs_rewrite, "兜底后需写回修正");
            }
            Loaded::UnsupportedVersion { .. } => {
                panic!("主题值非法不是版本问题，不该按版本不受支持处理")
            }
        }
    }

    /// 载荷缺字段：缺省即跟随系统与当前版本（写回时后端另强制版本号）。
    #[test]
    fn payload_defaults_missing_fields() {
        let payload: Preferences = serde_json::from_str("{}").unwrap();

        assert_eq!(payload.theme, Theme::System);
        assert_eq!(payload.version, PREFERENCES_VERSION);
    }

    /// 载荷的合法主题值原样进入存储。
    #[test]
    fn payload_carries_the_theme_verbatim() {
        let payload: Preferences = serde_json::from_str(r#"{"version":1,"theme":"dark"}"#).unwrap();

        assert_eq!(payload.theme, Theme::Dark);
    }
}
