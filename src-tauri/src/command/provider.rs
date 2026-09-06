use std::collections::BTreeMap;

use serde::Deserialize;
use tauri::State;

use crate::{
    keychain::{Keychain, Secret},
    lock_keychain, lock_store,
    provider::{Endpoints, ModelEntry, Provider},
    store::StoreError,
    AppStore,
};

/// 创建/更新 Provider 命令的 `provider` 负载。
///
/// `base_url` 与 `models` 缺省即视为未配置/空列表；`api_key` 的定制反序列化见
/// [`deserialize_api_key`]。
#[derive(Deserialize)]
pub struct ProviderRequest {
    #[serde(default)]
    base_url: Endpoints,
    #[serde(default, deserialize_with = "deserialize_api_key")]
    api_key: Option<Secret>,
    #[serde(default)]
    models: Vec<ModelEntry>,
}

/// `api_key` 字段的定制反序列化，接受 `{ slug, value? }` 对象（三态契约见 ADR 0002）：
///
/// - `api_key` 缺省或为 `null`：未设置凭证，反序列化为 `None`；
/// - `value` 缺省或为 `null`：不携带真值，反序列化为
///   `{ account: "provider/<slug>/api_key", value: None }`
///   （create 仅登记引用；update 解释为清除）；
/// - `value` 有值：反序列化为 `{ account: "provider/<slug>/api_key", value }`
///   （覆盖写入，真值待写入密钥链）。
///
/// 后两种形态均要求 `slug` 有值，缺失或为 `null` 即反序列化失败；
/// `api_key` 不是对象时同样失败——旧契约的裸字符串形态不再接受，
/// 防止把密钥引用或空串原样回传当作新凭证。
fn deserialize_api_key<'de, D>(deserializer: D) -> Result<Option<Secret>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    struct ApiKeyPayload {
        slug: String,
        #[serde(default)]
        value: Option<String>,
    }

    Ok(
        Option::<ApiKeyPayload>::deserialize(deserializer)?.map(|payload| {
            let slug = payload.slug;
            let account = format!("provider/{slug}/api_key");
            Secret::new(account, payload.value)
        }),
    )
}

/// 映射为配置记录：`api_key` 只落 [`Secret::reference`] 拼出的 `secret://` 引用，
/// 密钥真值不进入配置（ADR 0002 的单向写入姿态）。
impl From<ProviderRequest> for Provider {
    fn from(val: ProviderRequest) -> Self {
        Provider {
            base_url: val.base_url,
            api_key: val.api_key.map(|api_key| api_key.reference()),
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
    slug: String,
    provider: ProviderRequest,
) -> Result<(), String> {
    let mut guard = lock_store(&store)?;
    guard
        .create_provider(&slug, provider.into())
        .map_err(|e| e.message())
}

#[tauri::command]
pub fn update_provider(
    store: State<'_, AppStore>,
    slug: String,
    provider: ProviderRequest,
) -> Result<(), String> {
    let mut guard = lock_store(&store)?;
    guard
        .update_provider(&slug, provider.into())
        .map_err(|e| e.message())
}

#[tauri::command]
pub fn delete_provider(store: State<'_, AppStore>, slug: String) -> Result<Vec<String>, String> {
    let mut store_guard = lock_store(&store)?;
    let mut keychain_guard = lock_keychain(&store)?;
    let secret_references = store_guard
        .delete_provider(&slug)
        .map_err(|e| e.message())?;

    let mut warnings = Vec::new();
    for secret_reference in secret_references.into_iter().flatten() {
        if let Err(e) = keychain_guard.clear(&secret_reference) {
            warnings.push(format!(
                "已删除 Provider「{slug}」，但其密钥链条目清除失败：{}。该条目可能残留于系统密钥链。",
                e.detail()
            ));
        }
    }
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<ProviderRequest, serde_json::Error> {
        serde_json::from_str(text)
    }

    #[test]
    fn api_key_absent_or_null_deserializes_to_none() {
        assert_eq!(parse(r#"{}"#).unwrap().api_key, None);
        assert_eq!(parse(r#"{"api_key":null}"#).unwrap().api_key, None);
    }

    #[test]
    fn api_key_without_value_registers_bare_reference() {
        let request = parse(r#"{"api_key":{"slug":"openrouter"}}"#).unwrap();

        assert_eq!(
            request.api_key,
            Some(Secret::new("provider/openrouter/api_key".to_owned(), None))
        );
        assert_eq!(
            request.api_key.as_ref().unwrap().reference(),
            "secret://io.github.xezzon.agent-maestro/provider/openrouter/api_key"
        );

        let request = parse(r#"{"api_key":{"slug":"openrouter","value":null}}"#).unwrap();

        assert_eq!(
            request.api_key,
            Some(Secret::new("provider/openrouter/api_key".to_owned(), None))
        );
    }

    #[test]
    fn api_key_with_value_deserializes_to_secret_value() {
        let request = parse(r#"{"api_key":{"slug":"openrouter","value":"sk-test"}}"#).unwrap();

        assert_eq!(
            request.api_key,
            Some(Secret::new(
                "provider/openrouter/api_key".to_owned(),
                Some("sk-test".to_owned())
            ))
        );
    }

    #[test]
    fn api_key_object_without_slug_is_rejected() {
        assert!(parse(r#"{"api_key":{}}"#).is_err());
        assert!(parse(r#"{"api_key":{"value":"sk-test"}}"#).is_err());
        assert!(parse(r#"{"api_key":{"slug":null,"value":"sk-test"}}"#).is_err());
    }

    #[test]
    fn api_key_non_object_is_rejected() {
        assert!(parse(r#"{"api_key":"sk-test"}"#).is_err());
        assert!(parse(r#"{"api_key":42}"#).is_err());
    }

    #[test]
    fn provider_request_parses_full_payload() {
        let request = parse(
            r#"{
                "base_url": {"openai-completions": "http://127.0.0.1:8080/v1"},
                "api_key": {"slug": "openrouter", "value": "sk-test"},
                "models": [{"id": "gpt-4o", "display_name": "GPT-4o"}]
            }"#,
        )
        .unwrap();

        assert_eq!(
            request.base_url.openai_completions.as_deref(),
            Some("http://127.0.0.1:8080/v1")
        );
        assert_eq!(
            request.api_key,
            Some(Secret::new(
                "provider/openrouter/api_key".to_owned(),
                Some("sk-test".to_owned())
            ))
        );
        assert_eq!(request.models.len(), 1);
        assert_eq!(request.models[0].id, "gpt-4o");
    }

    #[test]
    fn provider_request_maps_to_record_with_reference() {
        let with_value: ProviderRequest =
            parse(r#"{"api_key":{"slug":"openrouter","value":"sk-test"}}"#).unwrap();

        let record: Provider = with_value.into();

        assert_eq!(
            record.api_key,
            Some("secret://io.github.xezzon.agent-maestro/provider/openrouter/api_key".to_owned()),
            "转换后配置里只落 secret:// 引用，密钥真值不得进入配置"
        );

        let bare_reference: ProviderRequest =
            parse(r#"{"api_key":{"slug":"openrouter"}}"#).unwrap();

        let record: Provider = bare_reference.into();

        assert_eq!(
            record.api_key,
            Some("secret://io.github.xezzon.agent-maestro/provider/openrouter/api_key".to_owned()),
            "仅登记 slug 时同样只落 secret:// 引用"
        );

        let without_api_key: ProviderRequest = parse(r#"{}"#).unwrap();

        let record: Provider = without_api_key.into();

        assert_eq!(record.api_key, None);
    }
}
