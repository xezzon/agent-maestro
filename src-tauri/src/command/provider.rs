use std::collections::BTreeMap;

use serde::Deserialize;
use tauri::State;

use crate::{
    AppStore, lock_store,
    provider::{Endpoints, ModelEntry, Provider},
    store::StoreError,
};

/// 创建/更新 Provider 命令的 `provider` 负载；slug 亦随负载传入。
///
/// `base_url` 与 `models` 缺省即视为未配置/空列表；`api_key` 缺省即未设置
/// （空串）。更新为整包替换，`api_key` 携带现值或新值（明文，第一期随配置
/// 文件落盘，ADR 0002 推迟采纳）。
#[derive(Deserialize)]
pub struct ProviderRequest {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    base_url: Endpoints,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    models: Vec<ModelEntry>,
}

/// 映射为配置记录：slug 由调用方另行取出作存储 key，不进记录本体；
/// `api_key` 缺省即落空串（未设置）。
impl From<ProviderRequest> for Provider {
    fn from(val: ProviderRequest) -> Self {
        Provider {
            base_url: val.base_url,
            api_key: val.api_key.unwrap_or_default(),
            models: val.models,
        }
    }
}

#[tauri::command]
pub fn list_providers(store: State<'_, AppStore>) -> Result<BTreeMap<String, Provider>, String> {
    let guard = lock_store(&store)?;
    let config = guard.get().map_err(StoreError::message)?;
    Ok(config.providers.clone())
}

#[tauri::command]
pub fn create_provider(
    store: State<'_, AppStore>,
    provider: ProviderRequest,
) -> Result<(), String> {
    let mut guard = lock_store(&store)?;

    let slug = provider.slug.clone();

    guard
        .create_provider(&slug, provider.into())
        .map_err(|e| e.message())
}

#[tauri::command]
pub fn update_provider(
    store: State<'_, AppStore>,
    provider: ProviderRequest,
) -> Result<(), String> {
    let mut guard = lock_store(&store)?;

    let slug = provider.slug.clone();

    guard
        .update_provider(&slug, provider.into())
        .map(|_| ())
        .map_err(|e| e.message())
}

#[tauri::command]
pub fn delete_provider(store: State<'_, AppStore>, slug: String) -> Result<(), String> {
    let mut guard = lock_store(&store)?;

    guard
        .delete_provider(&slug)
        .map(|_| ())
        .map_err(|e| e.message())
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
                "models": [{"id": "gpt-4o", "display_name": "GPT-4o"}]
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
}
