use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tauri::State;

use super::log_outcome;
use crate::{
    AppStore,
    errors::ProviderError,
    provider::{ApiKeyIntent, Endpoints, ModelEntry, Protocol, Provider, ProviderWrite},
    secret::{SCHEME, SecretRef, SecretText},
};

/// 创建/更新 Provider 命令的 `provider` 负载；slug 亦随负载传入。
///
/// `base_url` 与 `models` 缺省即视为未配置/空列表；`api_key` 的三种意图由
/// [`parse_api_key`] 解析（缺省/`null` = 不变、`""` = 清除、其余非空串 =
/// 覆盖）；`custom_header` 缺省即空映射；`selected_protocol` 缺省即未选择
/// （`null` 同义，见 ADR 0016）；`enabled` 缺省即启用（旧 payload 不带该字段时
/// 不被静默禁用）。模型条目随 `models` 携带 `limit` 与 `capabilities`。
/// 更新为整包替换，`enabled` 亦携带现值或新值。
/// 命令层不加值域校验：`limit` 三项用无符号整数承载，负数与非整数被**类型**
/// 拒绝，0 **被放行**（「正整数」只由前端 min=1 保证，见 issue #58）。
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
    custom_header: BTreeMap<String, String>,
    #[serde(default)]
    models: Vec<ModelEntry>,
    #[serde(default = "crate::provider::default_true")]
    enabled: bool,
}

/// 映射为配置记录：slug 由调用方另行取出作存储 key，不进记录本体；
/// `api_key` **不进记录**——凭证由写入意图决定，存储层按意图覆写该字段。
impl From<ProviderRequest> for Provider {
    fn from(val: ProviderRequest) -> Self {
        Provider {
            enabled: val.enabled,
            base_url: val.base_url,
            selected_protocol: val.selected_protocol,
            api_key: SecretText::unset(),
            custom_header: val.custom_header,
            models: val.models,
        }
    }
}

/// 解析 `api_key` 载荷：显式三态。
///
/// 缺省 / `null` = 不变；`""` = 清除；本 slug 的规范引用 = 不变（防前端
/// 回传）；其它 `secret://` 前缀值一律报错、不进存储层；其余非空串 = 覆盖。
///
/// 报错发生在命令层：引用只由 Maestro 自己写入，前端回传的一律不算凭证
/// （ADR 0002）。`secret://` 是保留前缀，明文密钥不能以它开头——真撞上了
/// 只能报错，绝不放行。
///
/// 规范引用归成「不变」是**更新**的规则；新建的 Provider 不该由引用起步，
/// 故新建路径先行拒绝任何引用（见 [`create_write`]）。
fn parse_api_key(slug: &str, payload: Option<&str>) -> Result<ApiKeyIntent, ProviderError> {
    let Some(text) = payload else {
        return Ok(ApiKeyIntent::Unchanged);
    };
    if text.is_empty() {
        return Ok(ApiKeyIntent::Clear);
    }
    if text.starts_with(SCHEME) {
        if SecretRef::is_canonical_for(slug, text) {
            return Ok(ApiKeyIntent::Unchanged);
        }
        return Err(ProviderError::unexpected_secret_reference(slug));
    }
    Ok(ApiKeyIntent::Overwrite(text.to_owned()))
}

/// 创建的一次写入：记录本体 + 凭证的写入意图。
///
/// 缺省与清除都落成未设置（新建没有旧值可沿用，也没有条目可删）；任何引用
/// （含规范引用）一律拒绝——新建的 Provider 不该由引用起步。
fn create_write(slug: &str, request: ProviderRequest) -> Result<ProviderWrite, ProviderError> {
    let payload = request.api_key.as_deref();
    if payload.is_some_and(|text| text.starts_with(SCHEME)) {
        return Err(ProviderError::unexpected_secret_reference(slug));
    }
    let intent = parse_api_key(slug, payload)?;
    let provider: Provider = request.into();
    Ok(match intent {
        // 新建没有条目可删：清除落成未设置。
        ApiKeyIntent::Unchanged | ApiKeyIntent::Clear => ProviderWrite::unchanged(provider),
        ApiKeyIntent::Overwrite(value) => ProviderWrite::overwrite(provider, value),
    })
}

/// 更新的一次写入：记录本体 + 凭证的写入意图。
///
/// 缺省与规范引用都按不变处理（规范引用是前端回传，防它产生新密钥）；空串即
/// 清除；其余非空串即覆盖。
fn update_write(slug: &str, request: ProviderRequest) -> Result<ProviderWrite, ProviderError> {
    let intent = parse_api_key(slug, request.api_key.as_deref())?;
    let provider: Provider = request.into();
    Ok(match intent {
        ApiKeyIntent::Unchanged => ProviderWrite::unchanged(provider),
        ApiKeyIntent::Clear => ProviderWrite::clear(provider),
        ApiKeyIntent::Overwrite(value) => ProviderWrite::overwrite(provider, value),
    })
}

/// 凭证状态（线格式）：未设置 / 已设置 / 明文待迁移。
///
/// 合法值由类型固定，线格式为小写词（`unset` / `set` / `plaintext`），前端按
/// 同一组词渲染标签。序列化形态与线格式由本模块的测试钉住：改词即前端标签
/// 静默失效。
///
/// 三态**完全由配置推导**：读列表不碰密钥链（#148）——真值在不在、还在不在
/// 配置文件里，配置自己说了算。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ApiKeyState {
    /// 未设置：没有凭证。
    Unset,
    /// 已设置：真值在系统密钥链里，配置里只留引用。
    Set,
    /// 明文待迁移：真值仍在配置文件里（启动迁移未成功，下次启动重试）。
    Plaintext,
}

impl From<&SecretText> for ApiKeyState {
    fn from(text: &SecretText) -> Self {
        match text {
            SecretText::Unset => Self::Unset,
            SecretText::Ref(_) => Self::Set,
            SecretText::Plaintext(_) => Self::Plaintext,
        }
    }
}

/// Provider 列表视图（命令返回给前端的形态）：**线格式契约**。
///
/// 与插件注册表的视图同理（见 `command/plugin.rs` 的 `PluginView`）：形状本来
/// 就是给界面看的，硬塞回领域反而更假，故它住在交付层。领域层给的是
/// [`Provider`]（凭证字段是 [`SecretText`] 形态），此处映射成界面要的形状。
///
/// 凭证**只以状态出现**：既不回传真值，也不回传引用 URI——单向写入的姿态
/// （ADR 0002）。
#[derive(Debug, Serialize)]
pub(crate) struct ProviderView {
    enabled: bool,
    base_url: Endpoints,
    selected_protocol: Option<Protocol>,
    /// 凭证状态（`unset` / `set` / `plaintext`）。
    api_key_state: ApiKeyState,
    custom_header: BTreeMap<String, String>,
    models: Vec<ModelEntry>,
}

impl From<Provider> for ProviderView {
    fn from(provider: Provider) -> Self {
        Self {
            enabled: provider.enabled,
            base_url: provider.base_url,
            selected_protocol: provider.selected_protocol,
            api_key_state: ApiKeyState::from(&provider.api_key),
            custom_header: provider.custom_header,
            models: provider.models,
        }
    }
}

/// 领域记录 → 视图：列表的线格式映射。
///
/// 凭证在映射这一步就变成状态：真值与引用 URI 都不进视图（ADR 0002）。
fn provider_views(providers: BTreeMap<String, Provider>) -> BTreeMap<String, ProviderView> {
    providers
        .into_iter()
        .map(|(slug, provider)| (slug, ProviderView::from(provider)))
        .collect()
}

#[tauri::command]
pub(crate) fn list_providers(
    store: State<'_, AppStore>,
) -> Result<BTreeMap<String, ProviderView>, String> {
    let outcome = store
        .with_read(|s| s.providers_snapshot())
        .map(provider_views)
        .map_err(|err| err.to_string());
    log_outcome("list_providers", None, &outcome);
    outcome
}

#[tauri::command]
pub(crate) fn create_provider(
    store: State<'_, AppStore>,
    provider: ProviderRequest,
) -> Result<(), String> {
    let slug = provider.slug.clone();
    let context = format!("slug={slug}");
    let outcome = create_write(&slug, provider)
        .and_then(|write| store.with_write(move |s| s.create_provider(&slug, write)))
        .map_err(|err| err.to_string());
    log_outcome("create_provider", Some(&context), &outcome);
    outcome
}

#[tauri::command]
pub(crate) fn update_provider(
    store: State<'_, AppStore>,
    provider: ProviderRequest,
) -> Result<(), String> {
    let slug = provider.slug.clone();
    let context = format!("slug={slug}");
    let outcome = update_write(&slug, provider)
        .and_then(|write| store.with_write(move |s| s.update_provider(&slug, write).map(|_| ())))
        .map_err(|err| err.to_string());
    log_outcome("update_provider", Some(&context), &outcome);
    outcome
}

#[tauri::command]
pub(crate) fn delete_provider(store: State<'_, AppStore>, slug: String) -> Result<(), String> {
    let outcome = store
        .with_write(|s| s.delete_provider(&slug).map(|_| ()))
        .map_err(|err| err.to_string());
    log_outcome("delete_provider", Some(&format!("slug={slug}")), &outcome);
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
    let outcome = store
        .with_write(|s| s.set_provider_enabled(&slug, enabled))
        .map_err(|err| err.to_string());
    log_outcome("set_provider_enabled", Some(&context), &outcome);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<ProviderRequest, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// 视图契约的线格式钉死：序列化出来就是 `unset` / `set` / `plaintext`
    /// 三个小写词。前端按同一组词渲染标签（见 `src/api/provider.js`），改词即
    /// 前端标签静默失效——故这三条断言是跨端契约的护栏。
    #[test]
    fn api_key_state_wire_values_are_pinned() {
        let cases = [
            (ApiKeyState::Unset, "unset"),
            (ApiKeyState::Set, "set"),
            (ApiKeyState::Plaintext, "plaintext"),
        ];

        for (state, wire) in cases {
            assert_eq!(
                serde_json::to_value(state).unwrap(),
                serde_json::json!(wire),
                "序列化形态与线格式词一致"
            );
        }
    }

    /// 视图只带状态：既不回传真值，也不回传引用 URI，更没有 `api_key` 字段
    /// （单向写入：渲染进程拿不到密钥，ADR 0002）。
    #[test]
    fn provider_view_carries_the_state_but_neither_value_nor_reference() {
        let cases = [
            (SecretText::unset(), "unset"),
            (
                SecretText::referenced(SecretRef::for_provider("openrouter").unwrap()),
                "set",
            ),
            (
                SecretText::Plaintext("sk-canary-do-not-leak".to_owned()),
                "plaintext",
            ),
        ];

        for (text, expected) in cases {
            let view = ProviderView::from(Provider {
                api_key: text,
                ..Provider::default()
            });
            let value = serde_json::to_value(&view).unwrap();

            assert_eq!(
                value["api_key_state"],
                serde_json::json!(expected),
                "状态词逐字对应"
            );
            assert!(
                value.get("api_key").is_none(),
                "视图里没有 api_key 字段：{value}"
            );
            assert!(
                !value.to_string().contains("secret://"),
                "视图不回传引用 URI：{value}"
            );
            assert!(
                !value.to_string().contains("sk-canary-do-not-leak"),
                "视图不回传真值：{value}"
            );
        }
    }

    /// 列表读取不触发任何密钥链调用：状态完全由配置推导（#148）。
    #[test]
    fn listing_providers_does_not_touch_the_keychain() {
        use crate::store::testutil::Fixture;

        let fixture = Fixture::new();
        let mut store = fixture.store();
        store
            .create_provider(
                "openrouter",
                ProviderWrite::overwrite(Provider::default(), "sk-live".to_owned()),
            )
            .unwrap();
        let app_store = AppStore::new(store, fixture.secrets().clone());
        let before = fixture.secrets().calls().len();

        let providers = app_store
            .with_read(|s| s.providers_snapshot())
            .map(provider_views)
            .unwrap();

        assert_eq!(
            fixture.secrets().calls().len(),
            before,
            "列表读取不碰密钥链：{:?}",
            fixture.secrets().calls()
        );
        assert_eq!(
            providers["openrouter"].api_key_state,
            ApiKeyState::Set,
            "状态仍由配置推导"
        );
    }

    /// 缺省与 `null` 都读作「不变」。
    #[test]
    fn api_key_absent_or_null_means_unchanged() {
        assert_eq!(parse(r#"{}"#).unwrap().api_key, None);
        assert_eq!(parse(r#"{"api_key":null}"#).unwrap().api_key, None);
        assert_eq!(
            create_write("openrouter", parse(r#"{}"#).unwrap())
                .unwrap()
                .api_key,
            ApiKeyIntent::Unchanged
        );
        assert_eq!(
            update_write("openrouter", parse(r#"{"api_key":null}"#).unwrap())
                .unwrap()
                .api_key,
            ApiKeyIntent::Unchanged
        );
    }

    /// 空串即清除：更新时删条目，新建时落成未设置。
    #[test]
    fn api_key_empty_string_means_clear() {
        let payload = parse(r#"{"api_key":""}"#).unwrap().api_key;
        assert_eq!(payload, Some(String::new()), "空串即清除凭证");

        assert_eq!(
            update_write("openrouter", parse(r#"{"api_key":""}"#).unwrap())
                .unwrap()
                .api_key,
            ApiKeyIntent::Clear
        );
        assert_eq!(
            create_write("openrouter", parse(r#"{"api_key":""}"#).unwrap())
                .unwrap()
                .api_key,
            ApiKeyIntent::Unchanged,
            "新建没有条目可删：空串落成未设置"
        );
    }

    /// 其余非空串即覆盖：真值随意图进密钥链，绝不落进配置。
    #[test]
    fn api_key_plain_string_is_the_new_plaintext_value() {
        let payload = parse(r#"{"api_key":"sk-test"}"#).unwrap().api_key;
        assert_eq!(payload.as_deref(), Some("sk-test"));

        assert_eq!(
            update_write("openrouter", parse(r#"{"api_key":"sk-test"}"#).unwrap())
                .unwrap()
                .api_key,
            ApiKeyIntent::Overwrite("sk-test".to_owned())
        );
        assert_eq!(
            create_write("openrouter", parse(r#"{"api_key":"sk-test"}"#).unwrap())
                .unwrap()
                .api_key,
            ApiKeyIntent::Overwrite("sk-test".to_owned())
        );
    }

    #[test]
    fn api_key_non_string_is_rejected() {
        assert!(parse(r#"{"api_key":{"slug":"openrouter"}}"#).is_err());
        assert!(parse(r#"{"api_key":42}"#).is_err());
    }

    /// 本 slug 的规范引用回传按「不变」处理：前端拿到的引用原样送回，
    /// 不产生新密钥、也不丢密钥。
    #[test]
    fn canonical_reference_is_treated_as_unchanged_on_update() {
        let canonical = "secret://io.github.xezzon.agent-maestro/provider/openrouter/api_key";

        assert_eq!(
            update_write(
                "openrouter",
                parse(&format!(r#"{{"api_key":"{canonical}"}}"#)).unwrap()
            )
            .unwrap()
            .api_key,
            ApiKeyIntent::Unchanged
        );
    }

    /// 新建时任何引用都拒绝：新建的 Provider 不该由引用起步。
    #[test]
    fn canonical_reference_is_rejected_on_create() {
        let canonical = "secret://io.github.xezzon.agent-maestro/provider/openrouter/api_key";

        let error = create_write(
            "openrouter",
            parse(&format!(r#"{{"api_key":"{canonical}"}}"#)).unwrap(),
        )
        .unwrap_err();

        assert!(
            matches!(error, ProviderError::UnexpectedSecretReference { .. }),
            "{error:?}"
        );
    }

    /// 别的 Provider 的引用、以及不成形的引用，一律报错、不进存储层。
    #[test]
    fn other_references_are_rejected() {
        let cases = [
            "secret://io.github.xezzon.agent-maestro/provider/other/api_key",
            "secret://io.github.xezzon.agent-maestro/provider/openrouter",
            "secret://elsewhere/provider/openrouter/api_key",
            "secret://",
        ];

        for payload in cases {
            let error = update_write(
                "openrouter",
                parse(&format!(r#"{{"api_key":"{payload}"}}"#)).unwrap(),
            )
            .unwrap_err();

            assert!(
                matches!(error, ProviderError::UnexpectedSecretReference { .. }),
                "payload={payload} error={error:?}"
            );
        }
    }

    /// 错误文案点名 Provider，且不携带载荷里的值。
    #[test]
    fn rejected_reference_message_names_the_provider_and_hides_the_payload() {
        let error = update_write(
            "openrouter",
            parse(r#"{"api_key":"secret://whatever"}"#).unwrap(),
        )
        .unwrap_err();

        let message = error.to_string();
        assert!(message.contains("openrouter"), "{message}");
        assert!(!message.contains("secret://whatever"), "{message}");
    }

    /// 规范引用回传端到端：前端拿到的引用原样送回，**不产生新密钥**、
    /// **不丢密钥**——旧值原样留在密钥链里，配置里仍是同一个引用。
    ///
    /// 这一条走真实的存储接缝（内存密钥链 + 临时配置文件）：命令层把引用归成
    /// 「不变」，存储层因此一个密钥链动作都不做。
    #[test]
    fn echoing_the_canonical_reference_back_leaves_the_keychain_untouched() {
        use crate::provider::{Provider, ProviderWrite};
        use crate::secret::SecretRef;
        use crate::store::testutil::Fixture;

        let fixture = Fixture::new();
        let mut store = fixture.store();
        store
            .create_provider(
                "openrouter",
                ProviderWrite::overwrite(Provider::default(), "sk-live".to_owned()),
            )
            .unwrap();
        let reference = SecretRef::for_provider("openrouter").unwrap();
        let before = fixture.secrets().calls().len();

        let write = update_write(
            "openrouter",
            parse(&format!(
                r#"{{"slug":"openrouter","api_key":"{reference}"}}"#
            ))
            .unwrap(),
        )
        .unwrap();
        store.update_provider("openrouter", write).unwrap();

        assert_eq!(
            fixture.secrets().calls().len(),
            before,
            "回传引用不产生新的密钥链动作：{:?}",
            fixture.secrets().calls()
        );
        assert_eq!(
            fixture.secrets().entry(&reference),
            Some("sk-live".to_owned()),
            "旧密钥原样留着"
        );
        let providers = store.providers_snapshot().unwrap();
        assert_eq!(
            providers["openrouter"]
                .api_key
                .reference()
                .map(SecretRef::slug),
            Some("openrouter"),
            "配置里仍是同一个引用"
        );
    }

    #[test]
    fn provider_request_parses_full_payload() {
        let request = parse(
            r#"{
                "slug": "openrouter",
                "base_url": {"openai-completions": "http://127.0.0.1:8080/v1"},
                "api_key": "sk-test",
                "custom_header": {"anthropic-version": "2023-06-01"},
                "models": [{
                    "id": "gpt-4o",
                    "display_name": "GPT-4o",
                    "limit": {"context_window": 128000, "max_output": 16384},
                    "capabilities": ["tool_use", "image_in"]
                }],
                "enabled": false
            }"#,
        )
        .unwrap();

        assert_eq!(request.slug, "openrouter");
        assert_eq!(
            request
                .base_url
                .get(&Protocol::OpenaiCompletions)
                .map(String::as_str),
            Some("http://127.0.0.1:8080/v1")
        );
        assert_eq!(request.api_key.as_deref(), Some("sk-test"));
        assert_eq!(request.custom_header["anthropic-version"], "2023-06-01");
        assert_eq!(request.models.len(), 1);
        assert_eq!(request.models[0].id, "gpt-4o");
        assert_eq!(
            request.models[0].limit,
            crate::provider::ModelLimit {
                context_window: Some(128_000),
                max_input: None,
                max_output: Some(16_384),
            }
        );
        assert_eq!(
            request.models[0].capabilities,
            vec![
                crate::provider::ModelCapability::ToolUse,
                crate::provider::ModelCapability::ImageIn
            ]
        );
        assert!(!request.enabled, "负载显式携带禁用态");
    }

    /// `custom_header` 缺省即空映射；显式映射原样进入记录。
    #[test]
    fn custom_header_absent_means_empty_map() {
        assert_eq!(parse(r#"{}"#).unwrap().custom_header, BTreeMap::new());

        let record: Provider = parse(r#"{"custom_header":{"X-Gateway-Key":"gw"}}"#)
            .unwrap()
            .into();

        assert_eq!(record.custom_header["X-Gateway-Key"], "gw");
    }

    /// `custom_header` 值非字符串被拒（键与值都是字符串的映射）。
    #[test]
    fn custom_header_non_string_value_is_rejected() {
        assert!(parse(r#"{"custom_header":{"X-Gateway-Key":42}}"#).is_err());
        assert!(parse(r#"{"custom_header":["X-Gateway-Key"]}"#).is_err());
    }

    /// `limit` 三项用无符号整数承载：0 被放行（正整数只由前端 min=1 保证），
    /// 负数与非整数被**类型**拒绝；缺省子项即未设置。
    #[test]
    fn model_limit_accepts_zero_and_rejects_negative_or_fractional() {
        let zero: Provider = parse(
            r#"{"models":[{"id":"m","limit":{"context_window":0,"max_input":0,"max_output":0}}]}"#,
        )
        .unwrap()
        .into();
        assert_eq!(
            zero.models[0].limit,
            crate::provider::ModelLimit {
                context_window: Some(0),
                max_input: Some(0),
                max_output: Some(0),
            },
            "0 被放行、原样进入记录"
        );

        assert!(
            parse(r#"{"models":[{"id":"m","limit":{"context_window":-1}}]}"#).is_err(),
            "负数被类型拒绝"
        );
        assert!(
            parse(r#"{"models":[{"id":"m","limit":{"max_output":1.5}}]}"#).is_err(),
            "非整数被类型拒绝"
        );
        assert!(
            parse(r#"{"models":[{"id":"m","limit":{"context_window":"128000"}}]}"#).is_err(),
            "字符串不被当作整数"
        );
    }

    #[test]
    fn model_limit_absent_subfields_are_unset() {
        let record: Provider = parse(r#"{"models":[{"id":"m"}]}"#).unwrap().into();

        assert_eq!(
            record.models[0].limit,
            crate::provider::ModelLimit::default()
        );
    }

    /// `capabilities` 取值受枚举约束：未知串与非字符串被拒。
    #[test]
    fn model_capabilities_reject_unknown_variants_and_non_strings() {
        assert!(parse(r#"{"models":[{"id":"m","capabilities":["telepathy"]}]}"#).is_err());
        assert!(parse(r#"{"models":[{"id":"m","capabilities":[1]}]}"#).is_err());
        assert!(parse(r#"{"models":[{"id":"m","capabilities":"tool_use"}]}"#).is_err());
    }

    /// `capabilities` 在负载反序列化时即归一（枚举序、去重），进入记录即稳定形状。
    #[test]
    fn model_capabilities_are_normalized_on_payload_parse() {
        let record: Provider =
            parse(r#"{"models":[{"id":"m","capabilities":["thinking","tool_use","thinking"]}]}"#)
                .unwrap()
                .into();

        assert_eq!(
            record.models[0].capabilities,
            vec![
                crate::provider::ModelCapability::ToolUse,
                crate::provider::ModelCapability::Thinking
            ]
        );
    }

    /// 凭证**不进记录**：真值只随写入意图流向密钥链，记录里只留形态。
    #[test]
    fn api_key_never_enters_the_record() {
        let with_value: Provider = parse(r#"{"api_key":"sk-test"}"#).unwrap().into();

        assert!(
            !with_value.api_key.is_set(),
            "记录里的 api_key 由存储层按意图覆写，命令层不填真值"
        );

        let without_api_key: Provider = parse(r#"{}"#).unwrap().into();

        assert!(!without_api_key.api_key.is_set(), "api_key 缺省即未设置");
    }

    #[test]
    fn update_request_maps_to_whole_record_including_api_key() {
        let write = update_write("openrouter", parse(r#"{"api_key":"sk-new"}"#).unwrap()).unwrap();

        assert_eq!(
            write.api_key,
            ApiKeyIntent::Overwrite("sk-new".to_owned()),
            "更新整包替换，api_key 携带新值"
        );
        assert!(!write.provider.api_key.is_set(), "记录里不落真值");
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
