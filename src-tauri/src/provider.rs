use std::{
    collections::BTreeMap,
    fmt::{Debug, Display},
};

use serde::{Deserialize, Serialize};

use crate::interpolate::interpolate_value;

/// Provider 在各协议下的端点（每协议至多一个；见 ADR 0003）。
///
/// 键缺省即未配置；显式空串原样保留（键在而值为空串），不做归一化。
pub(crate) type Endpoints = BTreeMap<Protocol, String>;

/// Provider 的线协议，与 WIT 合同的 `protocol` 一一对应；序列化为 kebab-case，
/// 与 [`Endpoints`] 的槽位键同名（`openai-completions` / `anthropic-messages`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Protocol {
    OpenaiCompletions,
    AnthropicMessages,
}

impl Display for Protocol {
    /// serde 线名（`openai-completions` / `anthropic-messages`），用于插值字段名。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Protocol::OpenaiCompletions => "openai-completions",
            Protocol::AnthropicMessages => "anthropic-messages",
        })
    }
}

/// Provider 下跨协议共享的一个模型条目。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ModelEntry {
    #[serde(default)]
    pub(crate) id: String,
    /// 无显示名时为 `None`，序列化为 `null`，界面回退显示 id。
    #[serde(default)]
    pub(crate) display_name: Option<String>,
    /// 声明式的 token 能力上限：**恒落盘**（缺省即 `{}`），子项未设置即不落盘。
    /// 只作声明：Maestro 不据此推断、不裁剪请求、不做跨字段一致性校验。
    #[serde(default)]
    pub(crate) limit: ModelLimit,
    /// 能力集合：**恒落盘**（缺省即 `[]`），按枚举声明序排序并去重。
    /// 读入与落盘都归一，手工改过的配置文件读回同样是稳定形状。
    #[serde(
        default,
        serialize_with = "serialize_capabilities",
        deserialize_with = "deserialize_capabilities"
    )]
    pub(crate) capabilities: Vec<ModelCapability>,
}

/// 模型的声明式 token 能力上限；三项互相独立、可缺省，未设置即不落盘。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ModelLimit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) context_window: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) max_input: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) max_output: Option<u32>,
}

/// 模型能力：声明顺序即落盘排序（`tool_use` → `image_in` → `thinking`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ModelCapability {
    ToolUse,
    ImageIn,
    Thinking,
}

/// 落盘前归一：按枚举声明序排序并去重（集合语义）。
fn serialize_capabilities<S>(
    capabilities: &[ModelCapability],
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let mut normalized = capabilities.to_vec();
    normalized.sort_unstable();
    normalized.dedup();
    normalized.serialize(serializer)
}

/// 读入时归一：手工写进配置文件的顺序/重复同样被规整。
fn deserialize_capabilities<'de, D>(deserializer: D) -> Result<Vec<ModelCapability>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let mut capabilities = Vec::<ModelCapability>::deserialize(deserializer)?;
    capabilities.sort_unstable();
    capabilities.dedup();
    Ok(capabilities)
}

/// 一条 LLM API 接入；以 slug 为 key 存于 providers 之下（见 CONTEXT.md）。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Provider {
    /// 是否参与投影：默认启用，旧配置缺该字段同样视为启用（向后兼容）。
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
    /// 按协议分槽的端点表：键缺省即未配置，显式空串原样保留（见 ADR 0003）。
    #[serde(default)]
    pub(crate) base_url: Endpoints,
    /// 界面所选、用于投影的协议（只存协议，端点 URL 不重复落盘）；`None` 即未选择，
    /// 不写入 JSON——旧配置没有该字段，读入即 `None`（向后兼容，见 ADR 0016）。
    /// 选择是否有效（该协议是否真的配了端点）由投影侧裁定。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) selected_protocol: Option<Protocol>,
    /// 凭证以明文随配置文件落盘：空串即未设置（第一期不做密钥链，
    /// ADR 0002 已修订为推迟采纳）。
    #[serde(default)]
    pub(crate) api_key: String,
    /// 跨协议、跨模型共享的自定义 HTTP header：**恒落盘**（缺省即 `{}`）。
    /// 键是标识符（与 api_key、模型 ID 同级，不参与插值）；值是配置值
    /// （参与宿主插值），且**可能含凭证**——Debug 只渲染键名，见下方手工 `Debug`。
    #[serde(default)]
    pub(crate) custom_header: BTreeMap<String, String>,
    /// 保序数组：模型 ID 不做字符集限制，且同一 Provider 内不重复（大小写敏感）。
    #[serde(default)]
    pub(crate) models: Vec<ModelEntry>,
}

/// `Provider::enabled` 的 serde 缺省值：启用（旧配置缺字段时向后兼容）。
/// `pub(crate)` 供 `command::provider` 的负载复用同一份缺省语义。
pub(crate) fn default_true() -> bool {
    true
}

/// 手工实现 `Default`：`enabled` 默认启用。测试夹具大量使用
/// `..Provider::default()`，派生的 `Default`（`enabled: false`）会让它们
/// 悄悄变成「不投影」，故必须显式给出启用态。
impl Default for Provider {
    fn default() -> Self {
        Self {
            enabled: true,
            base_url: Endpoints::default(),
            selected_protocol: None,
            api_key: String::new(),
            custom_header: BTreeMap::new(),
            models: Vec::new(),
        }
    }
}

/// Debug 视图：自定义 header 只渲染**键名**，绝不渲染任何值——值可能含凭证
/// （ADR 0008：凭证绝不落盘进日志）。
struct HeaderKeyNames<'a>(&'a BTreeMap<String, String>);

impl Debug for HeaderKeyNames<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.0.keys()).finish()
    }
}

/// 手工实现 Debug：api_key 渲染为 `<set>`/`<unset>`，绝不携带明文。
/// `Config` 的派生 `Debug` 内层调用它，因此嵌套在 Config 中的 Provider 同获保护
/// （ADR 0008：凭证绝不落盘进日志）。
impl Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider")
            .field("enabled", &self.enabled)
            .field("base_url", &self.base_url)
            .field("selected_protocol", &self.selected_protocol)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "<unset>"
                } else {
                    "<set>"
                },
            )
            .field("custom_header", &HeaderKeyNames(&self.custom_header))
            .field("models", &self.models)
            .finish()
    }
}

/// 插值失败：（字段名, 库报错）。库报错只含变量名与位置，不含值与密钥。
pub(crate) type FieldError = (String, String);

impl Provider {
    /// 插值（CONTEXT.md「插值」/ ADR 0015）：把自身全部字符串值里的占位符
    /// 替换为变量实际值，返回字面值副本；失败返回（字段名, 库报错）。
    ///
    /// 占位符语法与转义由 `crate::interpolate` 的纯函数执行（ADR 0014 换库哨兵）；
    /// 本方法只负责收集自身的字符串值。
    ///
    /// 作用域＝除 `api_key`、模型 `id` 与 header **键**外的一切字符串值（ADR 0015）。
    /// 端点表按协议遍历，新增协议自动纳入插值；不再有「新增字符串字段即编译失败」的哨兵，
    /// 但新增协议仍须补 `to_wit_protocol` 的穷举 match，否则插件侧收不到该端点。
    /// `enabled` 与 `selected_protocol` 是状态/枚举而非字符串值，原样透传、
    /// 不参与插值；`limit` 是数字、`capabilities` 是枚举，同样原样透传。
    pub(crate) fn interpolate(
        &self,
        variables: &BTreeMap<String, String>,
    ) -> Result<Provider, FieldError> {
        let mut base_url = BTreeMap::new();
        for (protocol, url) in &self.base_url {
            let field = format!("base_url.{protocol}");
            let value = interpolate_value(url, variables).map_err(|reason| (field, reason))?;
            base_url.insert(*protocol, value);
        }
        // api_key 原样保留，不参与插值。
        let api_key = self.api_key.clone();
        // header 的值参与插值；键是标识符（同 api_key、模型 ID），原样保留。
        let mut custom_header = BTreeMap::new();
        for (name, value) in &self.custom_header {
            let field = format!("custom_header.{name}");
            let value = interpolate_value(value, variables).map_err(|reason| (field, reason))?;
            custom_header.insert(name.clone(), value);
        }
        let mut models = Vec::with_capacity(self.models.len());
        for (index, model) in self.models.iter().enumerate() {
            let field = format!("models[{index}].display_name");
            let display_name = model
                .display_name
                .as_deref()
                .map(|name| interpolate_value(name, variables))
                .transpose()
                .map_err(|reason| (field, reason))?;
            // 模型 ID 是标识符：原样保留，不参与插值；limit 与 capabilities 亦非字符串值。
            models.push(ModelEntry {
                id: model.id.clone(),
                display_name,
                limit: model.limit,
                capabilities: model.capabilities.clone(),
            });
        }
        Ok(Provider {
            // enabled 是状态而非配置值：原样透传，不参与插值。
            enabled: self.enabled,
            base_url,
            // selected_protocol 是枚举选择而非字符串值：原样透传，不参与插值。
            selected_protocol: self.selected_protocol,
            api_key,
            custom_header,
            models,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_round_trip_explicit_empty_slots_verbatim() {
        let parsed: Endpoints = serde_json::from_str(r#"{"openai-completions":""}"#).unwrap();

        assert_eq!(
            parsed.get(&Protocol::OpenaiCompletions).map(String::as_str),
            Some("")
        );
        assert_eq!(
            serde_json::to_value(&parsed).unwrap(),
            serde_json::json!({"openai-completions": ""})
        );
    }

    #[test]
    fn provider_fields_default_when_absent() {
        let text = r#"{"base_url":{"anthropic-messages":"http://127.0.0.1:8080"}}"#;

        let parsed: Provider = serde_json::from_str(text).unwrap();

        assert_eq!(
            parsed
                .base_url
                .get(&Protocol::AnthropicMessages)
                .map(String::as_str),
            Some("http://127.0.0.1:8080")
        );
        assert!(
            !parsed.base_url.contains_key(&Protocol::OpenaiCompletions),
            "未配置的协议键缺省"
        );
        assert_eq!(parsed.api_key, "");
        assert!(parsed.models.is_empty());
        assert!(
            parsed.enabled,
            "旧配置缺 enabled 字段时视为启用（向后兼容）"
        );
    }

    /// 启用/禁用随记录落盘：`enabled` 恒被序列化，禁用态 round-trip 不丢。
    #[test]
    fn provider_round_trips_disabled_state() {
        let disabled = Provider {
            enabled: false,
            ..Provider::default()
        };

        let text = serde_json::to_string(&disabled).unwrap();
        assert!(
            text.contains(r#""enabled":false"#),
            "启用态是记录的一部分，必须显式落盘：{text}"
        );
        let parsed: Provider = serde_json::from_str(&text).unwrap();
        assert!(!parsed.enabled, "禁用态 round-trip 后仍是禁用");
    }

    /// 手工 `Default` 的契约：新建的 Provider 默认启用。
    #[test]
    fn default_provider_is_enabled() {
        assert!(
            Provider::default().enabled,
            "默认启用：新建与旧配置读入都不该落进禁用态"
        );
    }

    #[test]
    fn model_entry_serializes_id_and_nullable_display_name() {
        let entry = ModelEntry {
            id: "deepseek-chat".to_owned(),
            display_name: None,
            ..ModelEntry::default()
        };

        let text = serde_json::to_string(&entry).unwrap();

        assert_eq!(
            text,
            r#"{"id":"deepseek-chat","display_name":null,"limit":{},"capabilities":[]}"#
        );
        let parsed: ModelEntry = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn models_preserve_insertion_order() {
        let provider = Provider {
            models: vec![
                ModelEntry {
                    id: "z-model".to_owned(),
                    display_name: None,
                    ..ModelEntry::default()
                },
                ModelEntry {
                    id: "a-model".to_owned(),
                    display_name: Some("A Model".to_owned()),
                    ..ModelEntry::default()
                },
            ],
            ..Provider::default()
        };

        let text = serde_json::to_string(&provider).unwrap();
        let z = text.find("\"z-model\"").unwrap();
        let a = text.find("\"a-model\"").unwrap();

        assert!(z < a, "models 必须按插入顺序序列化为数组");
    }

    #[test]
    fn provider_round_trips_dual_endpoints_plaintext_api_key_and_models() {
        let provider = Provider {
            enabled: true,
            selected_protocol: None,
            base_url: BTreeMap::from([
                (
                    Protocol::OpenaiCompletions,
                    "https://api.example.com/v1".to_owned(),
                ),
                (
                    Protocol::AnthropicMessages,
                    "https://anthropic.example.com/v1".to_owned(),
                ),
            ]),
            api_key: "sk-test".to_owned(),
            custom_header: BTreeMap::from([
                ("X-Gateway-Key".to_owned(), "gw-plain".to_owned()),
                ("anthropic-version".to_owned(), "2023-06-01".to_owned()),
            ]),
            models: vec![ModelEntry {
                id: "gpt-4o".to_owned(),
                display_name: Some("GPT-4o".to_owned()),
                limit: ModelLimit {
                    context_window: Some(128_000),
                    max_input: None,
                    max_output: Some(16_384),
                },
                // 已归一（枚举序、去重）的集合：round-trip 后逐字节相同。
                capabilities: vec![ModelCapability::ToolUse, ModelCapability::ImageIn],
            }],
        };

        let parsed: Provider =
            serde_json::from_str(&serde_json::to_string(&provider).unwrap()).unwrap();

        assert_eq!(parsed, provider);
    }

    /// 协议枚举的线格式与端点槽位键一致，前端可直接用协议名查端点。
    #[test]
    fn protocol_serializes_as_the_endpoint_slot_key() {
        assert_eq!(
            serde_json::to_value(Protocol::OpenaiCompletions).unwrap(),
            serde_json::json!("openai-completions")
        );
        assert_eq!(
            serde_json::to_value(Protocol::AnthropicMessages).unwrap(),
            serde_json::json!("anthropic-messages")
        );
    }

    /// 选择是记录的一部分：round-trip 不丢；旧配置没有该字段时读作未选择，
    /// 且未选择时不落盘（保持既有配置形状，见 ADR 0016）。
    #[test]
    fn selected_protocol_round_trips_and_defaults_to_none() {
        let selected = Provider {
            base_url: BTreeMap::from([
                (
                    Protocol::OpenaiCompletions,
                    "https://a.example.com/v1".to_owned(),
                ),
                (
                    Protocol::AnthropicMessages,
                    "https://b.example.com".to_owned(),
                ),
            ]),
            selected_protocol: Some(Protocol::AnthropicMessages),
            ..Provider::default()
        };

        let text = serde_json::to_string(&selected).unwrap();
        assert!(
            text.contains(r#""selected_protocol":"anthropic-messages""#),
            "选择随记录落盘：{text}"
        );
        assert_eq!(
            serde_json::from_str::<Provider>(&text).unwrap(),
            selected,
            "选择 round-trip 后不变"
        );

        let absent: Provider = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(absent.selected_protocol, None, "缺字段即未选择");
        assert!(
            !serde_json::to_string(&Provider::default())
                .unwrap()
                .contains("selected_protocol"),
            "未选择时不落盘"
        );
    }

    #[test]
    fn unknown_selected_protocol_is_rejected() {
        assert!(serde_json::from_str::<Provider>(r#"{"selected_protocol":"grpc"}"#).is_err());
    }

    #[test]
    fn unset_api_key_is_always_written_as_empty_string() {
        let text = serde_json::to_string(&Provider::default()).unwrap();

        assert!(
            text.contains(r#""api_key":""#),
            "api_key 恒为明文字符串，空串即未设置（见 #24 数据模型）"
        );
        let parsed: Provider = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.api_key, "");
    }

    /// L1 红线（ADR 0008）：Debug 渲染绝不携带 api_key 明文，只保留
    /// 「配没配 key」这一排障高频信息。
    #[test]
    fn debug_output_never_contains_the_api_key() {
        let set = Provider {
            api_key: "sk-secret-canary-9f3a".to_owned(),
            ..Provider::default()
        };
        let rendered = format!("{set:?}");

        assert!(
            !rendered.contains("sk-secret-canary-9f3a"),
            "Debug 输出不得包含 api_key：{rendered}"
        );
        assert!(rendered.contains("<set>"), "已配置渲染为 <set>：{rendered}");
        assert!(
            format!("{:?}", Provider::default()).contains("<unset>"),
            "未配置渲染为 <unset>"
        );
    }

    fn vars(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    /// 插值（`Provider::interpolate`）输出全部字面值副本：除模型 ID、api_key 与
    /// header 键（原样保留）外逐字段替换，原值不动。
    #[test]
    fn interpolate_replaces_strings_field_by_field_and_keeps_the_original() {
        let provider = Provider {
            enabled: true,
            base_url: BTreeMap::from([(
                Protocol::OpenaiCompletions,
                "https://${HOST}/v1".to_owned(),
            )]),
            selected_protocol: Some(Protocol::OpenaiCompletions),
            api_key: "${KEY}".to_owned(),
            // header 的值参与插值；键里的占位符是标识符，原样保留。
            custom_header: BTreeMap::from([
                ("X-Gateway-Key".to_owned(), "${KEY}".to_owned()),
                ("${KEY}".to_owned(), "literal".to_owned()),
            ]),
            models: vec![
                ModelEntry {
                    id: "${MODEL}".to_owned(),
                    display_name: Some("Model ${MODEL}".to_owned()),
                    limit: ModelLimit {
                        context_window: Some(200_000),
                        max_input: None,
                        max_output: None,
                    },
                    capabilities: vec![ModelCapability::Thinking],
                },
                ModelEntry {
                    id: "plain-id".to_owned(),
                    display_name: None,
                    ..ModelEntry::default()
                },
            ],
        };

        let variables = vars(&[
            ("HOST", "api.example.com"),
            ("KEY", "sk-x"),
            ("MODEL", "gpt-4o"),
        ]);
        let projected = provider.interpolate(&variables).unwrap();

        assert_eq!(
            projected
                .base_url
                .get(&Protocol::OpenaiCompletions)
                .map(String::as_str),
            Some("https://api.example.com/v1")
        );
        assert_eq!(
            projected.selected_protocol,
            Some(Protocol::OpenaiCompletions),
            "选择是枚举，原样投影、不参与插值"
        );
        assert_eq!(projected.api_key, "${KEY}", "api_key 原样投影、不参与插值");
        assert_eq!(
            projected.custom_header["X-Gateway-Key"], "sk-x",
            "header 值是配置值，参与插值"
        );
        assert_eq!(
            projected.custom_header["${KEY}"], "literal",
            "header 键是标识符，不参与插值"
        );
        assert_eq!(
            projected.models[0].id, "${MODEL}",
            "模型 ID 是标识符，原样投影、不参与插值"
        );
        assert_eq!(
            projected.models[0].display_name.as_deref(),
            Some("Model gpt-4o")
        );
        assert_eq!(
            projected.models[0].limit,
            ModelLimit {
                context_window: Some(200_000),
                ..ModelLimit::default()
            },
            "limit 是数字，原样投影、不参与插值"
        );
        assert_eq!(
            projected.models[0].capabilities,
            vec![ModelCapability::Thinking],
            "capabilities 是枚举，原样投影、不参与插值"
        );
        assert_eq!(projected.models[1].id, "plain-id");
        assert_eq!(projected.models[1].display_name, None);
        // 原 Provider 不被修改。
        assert_eq!(provider.api_key, "${KEY}");
    }

    /// 任一字段失败即整体失败：返回（字段名, 库报错），不含值与密钥。
    #[test]
    fn interpolate_failure_reports_field_and_variable_without_values() {
        let provider = Provider {
            base_url: BTreeMap::from([(
                Protocol::OpenaiCompletions,
                "${UNDEFINED_VAR}".to_owned(),
            )]),
            ..Provider::default()
        };

        let (field, reason) = provider.interpolate(&vars(&[])).unwrap_err();

        assert_eq!(field, "base_url.openai-completions");
        assert!(reason.contains("UNDEFINED_VAR"), "{reason}");
    }

    /// custom_header 是记录的一部分且**恒落盘**：未设置时写出空映射，
    /// 旧配置（无该字段）读入即空映射。
    #[test]
    fn custom_header_is_always_written_and_legacy_files_read_empty() {
        let text = serde_json::to_string(&Provider::default()).unwrap();
        assert!(
            text.contains(r#""custom_header":{}"#),
            "未设置时仍写出空映射：{text}"
        );

        let legacy: Provider = serde_json::from_str(r#"{"api_key":"sk-legacy"}"#).unwrap();
        assert_eq!(legacy.custom_header, BTreeMap::new(), "旧配置读作空映射");
        assert_eq!(legacy.api_key, "sk-legacy");
    }

    /// header 名按 BTreeMap 键序落盘（映射本无序，排序保证产物确定）。
    #[test]
    fn custom_header_entries_round_trip_sorted_by_name() {
        let provider = Provider {
            custom_header: BTreeMap::from([
                ("X-Zeta".to_owned(), "z".to_owned()),
                ("X-Alpha".to_owned(), "a".to_owned()),
            ]),
            ..Provider::default()
        };

        let text = serde_json::to_string(&provider).unwrap();
        let alpha = text.find("X-Alpha").unwrap();
        let zeta = text.find("X-Zeta").unwrap();
        assert!(alpha < zeta, "键序稳定：{text}");

        let parsed: Provider = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, provider);
    }

    /// L1 红线（ADR 0008）：Debug 只渲染 header 键名，绝不携带任何值。
    #[test]
    fn debug_output_never_contains_custom_header_values() {
        let provider = Provider {
            custom_header: BTreeMap::from([(
                "X-Gateway-Key".to_owned(),
                "gw-secret-canary-4d21".to_owned(),
            )]),
            ..Provider::default()
        };

        let rendered = format!("{provider:?}");

        assert!(
            !rendered.contains("gw-secret-canary-4d21"),
            "Debug 输出不得包含 header 值：{rendered}"
        );
        assert!(
            rendered.contains("X-Gateway-Key"),
            "只保留键名这一排障信息：{rendered}"
        );
    }

    /// header 插值失败：字段名用键名（标识符），不携带值。
    #[test]
    fn interpolate_failure_for_header_reports_key_only() {
        let provider = Provider {
            custom_header: BTreeMap::from([(
                "X-Gateway-Key".to_owned(),
                "${UNDEFINED_VAR}".to_owned(),
            )]),
            ..Provider::default()
        };

        let (field, reason) = provider.interpolate(&vars(&[])).unwrap_err();

        assert_eq!(field, "custom_header.X-Gateway-Key");
        assert!(reason.contains("UNDEFINED_VAR"), "{reason}");
    }

    /// limit 恒落盘（缺省即 `{}`），未设置的子项不落盘。
    #[test]
    fn model_limit_is_always_written_and_omits_unset_subfields() {
        let unset = serde_json::to_string(&ModelEntry::default()).unwrap();
        assert!(
            unset.contains(r#""limit":{}"#),
            "未设置时仍写出空对象：{unset}"
        );

        let partial = ModelEntry {
            id: "gpt-4o".to_owned(),
            limit: ModelLimit {
                context_window: Some(128_000),
                max_input: None,
                max_output: Some(16_384),
            },
            ..ModelEntry::default()
        };

        let text = serde_json::to_string(&partial).unwrap();

        assert_eq!(
            text,
            r#"{"id":"gpt-4o","display_name":null,"limit":{"context_window":128000,"max_output":16384},"capabilities":[]}"#,
            "只落盘已设置的子项"
        );
        assert_eq!(serde_json::from_str::<ModelEntry>(&text).unwrap(), partial);
    }

    /// 旧配置缺新字段：limit 读作未设置、capabilities 读作空集合。
    #[test]
    fn legacy_model_entry_without_new_fields_reads_as_unset() {
        let entry: ModelEntry =
            serde_json::from_str(r#"{"id":"gpt-4o","display_name":"GPT-4o"}"#).unwrap();

        assert_eq!(entry.limit, ModelLimit::default());
        assert!(entry.capabilities.is_empty());
    }

    /// capabilities 恒落盘（缺省即 `[]`），落盘前按枚举声明序排序并去重。
    #[test]
    fn capabilities_are_always_written_sorted_and_deduplicated() {
        assert!(
            serde_json::to_string(&ModelEntry::default())
                .unwrap()
                .contains(r#""capabilities":[]"#),
            "未设置时仍写出空数组"
        );

        let entry = ModelEntry {
            capabilities: vec![
                ModelCapability::Thinking,
                ModelCapability::ToolUse,
                ModelCapability::Thinking,
                ModelCapability::ImageIn,
            ],
            ..ModelEntry::default()
        };

        let text = serde_json::to_string(&entry).unwrap();

        assert!(
            text.contains(r#""capabilities":["tool_use","image_in","thinking"]"#),
            "落盘按枚举声明序、去重：{text}"
        );
    }

    /// 手工写进配置文件的顺序与重复在**读入**时同样归一（读回即稳定形状）。
    #[test]
    fn hand_edited_capabilities_are_normalized_on_read() {
        let text = r#"{"id":"m","capabilities":["thinking","tool_use","thinking","image_in"]}"#;

        let parsed: ModelEntry = serde_json::from_str(text).unwrap();

        assert_eq!(
            parsed.capabilities,
            vec![
                ModelCapability::ToolUse,
                ModelCapability::ImageIn,
                ModelCapability::Thinking
            ]
        );
        assert!(
            serde_json::to_string(&parsed)
                .unwrap()
                .contains(r#""capabilities":["tool_use","image_in","thinking"]"#),
            "读入归一后回写同一形状"
        );
    }

    #[test]
    fn unknown_capability_is_rejected() {
        assert!(serde_json::from_str::<ModelEntry>(r#"{"capabilities":["telepathy"]}"#).is_err());
    }
}
