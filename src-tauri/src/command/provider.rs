use std::collections::BTreeMap;

use serde::Deserialize;
use tauri::State;

use super::log_outcome;
use crate::{
    AppStore,
    provider::{Endpoints, ModelEntry, Protocol, Provider},
};

/// 创建/更新 Provider 命令的 `provider` 负载；slug 亦随负载传入。
///
/// `base_url` 与 `models` 缺省即视为未配置/空列表；`api_key` 缺省即未设置
/// （空串）；`selected_protocol` 缺省即未选择（`null` 同义，见 ADR 0016）；
/// `enabled` 缺省即启用（旧 payload 不带该字段时不被静默禁用）。
/// 更新为整包替换，`api_key` 携带现值或新值（明文，第一期随配置
/// 文件落盘，ADR 0002 推迟采纳），`enabled` 亦携带现值或新值。
#[derive(Deserialize)]
pub(crate) struct ProviderRequest {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    base_url: Endpoints,
    #[serde(default)]
    selected_protocol: Option<Protocol>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    models: Vec<ModelEntry>,
    #[serde(default = "crate::provider::default_true")]
    enabled: bool,
}

/// 映射为配置记录：slug 由调用方另行取出作存储 key，不进记录本体；
/// `api_key` 缺省即落空串（未设置）。
impl From<ProviderRequest> for Provider {
    fn from(val: ProviderRequest) -> Self {
        Provider {
            enabled: val.enabled,
            base_url: val.base_url,
            selected_protocol: val.selected_protocol,
            api_key: val.api_key.unwrap_or_default(),
            models: val.models,
        }
    }
}

#[tauri::command]
pub(crate) fn list_providers(
    store: State<'_, AppStore>,
) -> Result<BTreeMap<String, Provider>, String> {
    let outcome = (|| -> Result<BTreeMap<String, Provider>, String> {
        let guard = store.read()?;
        let config = guard.get()?;
        Ok(config.providers.clone())
    })();
    log_outcome("list_providers", "", &outcome);
    outcome
}

#[tauri::command]
pub(crate) fn create_provider(
    store: State<'_, AppStore>,
    provider: ProviderRequest,
) -> Result<(), String> {
    let slug = provider.slug.clone();
    let context = format!("slug={slug}");
    let outcome = (move || {
        let mut guard = store.write()?;
        Ok(guard.create_provider(&slug, provider.into())?)
    })();
    log_outcome("create_provider", &context, &outcome);
    outcome
}

#[tauri::command]
pub(crate) fn update_provider(
    store: State<'_, AppStore>,
    provider: ProviderRequest,
) -> Result<(), String> {
    let slug = provider.slug.clone();
    let context = format!("slug={slug}");
    let outcome = (move || {
        let mut guard = store.write()?;
        guard.update_provider(&slug, provider.into())?;
        Ok(())
    })();
    log_outcome("update_provider", &context, &outcome);
    outcome
}

#[tauri::command]
pub(crate) fn delete_provider(store: State<'_, AppStore>, slug: String) -> Result<(), String> {
    let outcome = (|| {
        let mut guard = store.write()?;
        guard.delete_provider(&slug)?;
        Ok(())
    })();
    log_outcome("delete_provider", &format!("slug={slug}"), &outcome);
    outcome
}

/// 切换单个 Provider 的启用状态（不经过编辑表单的独立开关）。
#[tauri::command]
pub(crate) fn set_provider_enabled(
    store: State<'_, AppStore>,
    slug: String,
    enabled: bool,
) -> Result<(), String> {
    let context = format!("slug={slug} enabled={enabled}");
    let outcome = (|| {
        let mut guard = store.write()?;
        guard.set_provider_enabled(&slug, enabled)?;
        Ok(())
    })();
    log_outcome("set_provider_enabled", &context, &outcome);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<ProviderRequest, serde_json::Error> {
        serde_json::from_str(text)
    }

    #[test]
    fn api_key_absent_or_null_means_unset() {
        assert_eq!(parse(r#"{}"#).unwrap().api_key, None);
        assert_eq!(parse(r#"{"api_key":null}"#).unwrap().api_key, None);
    }

    #[test]
    fn api_key_empty_string_means_clear() {
        assert_eq!(
            parse(r#"{"api_key":""}"#).unwrap().api_key,
            Some(String::new()),
            "空串即清除凭证"
        );
    }

    #[test]
    fn api_key_plain_string_is_the_new_plaintext_value() {
        assert_eq!(
            parse(r#"{"api_key":"sk-test"}"#).unwrap().api_key,
            Some("sk-test".to_owned())
        );
    }

    #[test]
    fn api_key_non_string_is_rejected() {
        assert!(parse(r#"{"api_key":{"slug":"openrouter"}}"#).is_err());
        assert!(parse(r#"{"api_key":42}"#).is_err());
    }

    #[test]
    fn provider_request_parses_full_payload() {
        let request = parse(
            r#"{
                "slug": "openrouter",
                "base_url": {"openai-completions": "http://127.0.0.1:8080/v1"},
                "api_key": "sk-test",
                "models": [{"id": "gpt-4o", "display_name": "GPT-4o"}],
                "enabled": false
            }"#,
        )
        .unwrap();

        assert_eq!(request.slug, "openrouter");
        assert_eq!(
            request.base_url.openai_completions.as_deref(),
            Some("http://127.0.0.1:8080/v1")
        );
        assert_eq!(request.api_key.as_deref(), Some("sk-test"));
        assert_eq!(request.models.len(), 1);
        assert_eq!(request.models[0].id, "gpt-4o");
        assert!(!request.enabled, "负载显式携带禁用态");
    }

    #[test]
    fn create_request_maps_to_record_with_plaintext_api_key() {
        let with_value: Provider = parse(r#"{"api_key":"sk-test"}"#).unwrap().into();

        assert_eq!(
            with_value.api_key, "sk-test",
            "凭证以明文直接进入配置记录（第一期不做密钥链）"
        );

        let without_api_key: Provider = parse(r#"{}"#).unwrap().into();

        assert_eq!(without_api_key.api_key, "", "api_key 缺省即未设置（空串）");
    }

    #[test]
    fn update_request_maps_to_whole_record_including_api_key() {
        let with_value: Provider = parse(r#"{"api_key":"sk-new"}"#).unwrap().into();

        assert_eq!(
            with_value.api_key, "sk-new",
            "更新整包替换，api_key 携带现值或新值（明文）"
        );

        let without_api_key: Provider = parse(r#"{}"#).unwrap().into();

        assert_eq!(without_api_key.api_key, "", "api_key 缺省即未设置（空串）");
    }

    /// `enabled` 缺省即启用：旧 payload 或漏字段不会被静默禁用。
    #[test]
    fn enabled_absent_means_enabled() {
        assert!(parse(r#"{}"#).unwrap().enabled);
        let record: Provider = parse(r#"{}"#).unwrap().into();
        assert!(record.enabled, "缺省映射为启用");
    }

    /// 显式携带的禁用态原样进入记录，不被缺省值覆盖。
    #[test]
    fn enabled_false_is_mapped_verbatim() {
        let record: Provider = parse(r#"{"enabled":false}"#).unwrap().into();

        assert!(!record.enabled);
    }

    /// 选择缺省（或显式 `null`）即未选择；显式协议原样进入记录（ADR 0016）。
    #[test]
    fn selected_protocol_absent_is_none_and_explicit_is_mapped() {
        assert_eq!(parse(r#"{}"#).unwrap().selected_protocol, None);
        assert_eq!(
            parse(r#"{"selected_protocol":null}"#)
                .unwrap()
                .selected_protocol,
            None
        );

        let record: Provider = parse(r#"{"selected_protocol":"anthropic-messages"}"#)
            .unwrap()
            .into();

        assert_eq!(record.selected_protocol, Some(Protocol::AnthropicMessages));
    }
}
