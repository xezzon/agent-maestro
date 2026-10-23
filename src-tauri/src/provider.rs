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

impl ModelEntry {
    /// 插值（ADR 0015）：作用域＝本条目参与插值的字符串值，当前只有 `display_name`。
    /// 模型 ID 是标识符、原样保留；`limit` 与 `capabilities` 非字符串值、原样透传。
    /// 返回的字段名以本条目为根（`display_name`），所处的数组下标由调用方补齐。
    pub(crate) fn interpolate(
        &self,
        variables: &BTreeMap<String, String>,
    ) -> Result<ModelEntry, FieldError> {
        let display_name = self
            .display_name
            .as_deref()
            .map(|name| interpolate_value(name, variables))
            .transpose()
            .map_err(|reason| FieldError {
                field: "display_name".to_owned(),
                reason,
            })?;
        Ok(ModelEntry {
            id: self.id.clone(),
            display_name,
            limit: self.limit,
            capabilities: self.capabilities.clone(),
        })
    }
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

/// Provider 的凭证：未设置，或一段非空明文。
///
/// 「未设置」与「空串」不再混淆：空串不是合法的凭证值，本类型造不出那种
/// 状态——命令层的缺省载荷与显式空串都归一为「未设置」，规则只在这里一份
/// （此前散在领域层的 `is_empty()`、命令层的 `unwrap_or_default()` 与前端
/// 的 `!!api_key` 三处）。
///
/// 脱敏是类型级隔离：[`Debug`] 只渲染「配没配」，绝不渲染明文——调用点不必
/// 记得别打印，派生 `Debug` 的容器（如 `Config`）自动安全。
#[derive(Clone, PartialEq, Eq, Default)]
pub(crate) struct ApiKey {
    /// 明文凭证；`None` 即未设置。
    value: Option<String>,
}

impl ApiKey {
    /// 未设置。
    pub(crate) fn unset() -> Self {
        Self { value: None }
    }

    /// 由一段明文构造：空串归一为未设置（空串不是合法凭证值）。
    pub(crate) fn new(value: String) -> Self {
        match value.is_empty() {
            true => Self::unset(),
            false => Self { value: Some(value) },
        }
    }

    /// 由命令层的可选载荷构造：缺省与空串都读作未设置。
    pub(crate) fn from_optional(value: Option<String>) -> Self {
        match value {
            Some(value) => Self::new(value),
            None => Self::unset(),
        }
    }

    /// 明文凭证；`None` 即未设置。
    pub(crate) fn as_deref(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// 是否已配置凭证。
    pub(crate) fn is_set(&self) -> bool {
        self.value.is_some()
    }
}

impl Debug for ApiKey {
    /// 只渲染「配没配」，绝不渲染明文（ADR 0008：凭证绝不落盘进日志）。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.is_set() { "<set>" } else { "<unset>" })
    }
}

impl Serialize for ApiKey {
    /// 存储形态不变：明文字符串，未设置即空串（ADR 0002 推迟采纳密钥链）。
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.value.as_deref().unwrap_or(""))
    }
}

impl<'de> Deserialize<'de> for ApiKey {
    /// 读入：空串即未设置（旧配置文件里未设置的凭证恒为空串）。
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(ApiKey::new(raw))
    }
}

/// 一条 LLM API 接入；以 slug 为 key 存于 providers 之下（见 docs/GLOSSARY.md）。
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
    /// 凭证以明文随配置文件落盘：未设置即空串（第一期不做密钥链，
    /// ADR 0002 已修订为推迟采纳）。「未设置」由 [`ApiKey`] 独家表达。
    #[serde(default)]
    pub(crate) api_key: ApiKey,
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
            api_key: ApiKey::default(),
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
            // api_key 的脱敏由 `ApiKey` 自己负责：类型级隔离，调用点不必
            // 记得别打印明文。
            .field("api_key", &self.api_key)
            .field("custom_header", &HeaderKeyNames(&self.custom_header))
            .field("models", &self.models)
            .finish()
    }
}

/// 插值失败：字段名 + 原因。
///
/// 字段名指明插的是哪个字段（`base_url.openai-completions`、`models[1].display_name`），
/// 原因取自库（只含变量名与位置，不含值与密钥）。
///
/// 用具名字段而非匿名元组：元组的两个位置写反了编译器不报错，解构点读到的
/// 字段名与原因会悄悄对调。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FieldError {
    /// 出错的字段名。
    pub(crate) field: String,
    /// 原因（库报错）。
    pub(crate) reason: String,
}

impl Provider {
    /// 插值（docs/GLOSSARY.md「插值」/ ADR 0015）：把自身全部字符串值里的占位符
    /// 替换为变量实际值，返回字面值副本；失败返回（字段名, 库报错）。
    ///
    /// 占位符语法与转义由 `crate::interpolate` 的纯函数执行（ADR 0014 换库哨兵）；
    /// 本方法负责收集自身的字符串值，`models` 的字符串值交由
    /// [`ModelEntry::interpolate`] 逐条插值、本方法只补数组下标路径。
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
            let value =
                interpolate_value(url, variables).map_err(|reason| FieldError { field, reason })?;
            base_url.insert(*protocol, value);
        }
        // api_key 原样保留，不参与插值。
        let api_key = self.api_key.clone();
        // header 的值参与插值；键是标识符（同 api_key、模型 ID），原样保留。
        let mut custom_header = BTreeMap::new();
        for (name, value) in &self.custom_header {
            let field = format!("custom_header.{name}");
            let value = interpolate_value(value, variables)
                .map_err(|reason| FieldError { field, reason })?;
            custom_header.insert(name.clone(), value);
        }
        let mut models = Vec::with_capacity(self.models.len());
        for (index, model) in self.models.iter().enumerate() {
            models.push(model.interpolate(variables).map_err(|error| FieldError {
                field: format!("models[{index}].{}", error.field),
                reason: error.reason,
            })?);
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
        assert!(!parsed.api_key.is_set(), "api_key 未设置");
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
            api_key: ApiKey::new("sk-test".to_owned()),
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
        assert!(!parsed.api_key.is_set(), "api_key 未设置");
    }

    /// L1 红线（ADR 0008）：Debug 渲染绝不携带 api_key 明文，只保留
    /// 「配没配 key」这一排障高频信息。
    #[test]
    fn debug_output_never_contains_the_api_key() {
        let set = Provider {
            api_key: ApiKey::new("sk-secret-canary-9f3a".to_owned()),
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

    /// 凭证的两种状态在类型上分清：未设置与已设置。空串不是合法凭证值——
    /// 造不出那种状态（命令层的缺省载荷与显式空串都归一为未设置）。
    #[test]
    fn api_key_distinguishes_unset_from_set_and_never_holds_an_empty_string() {
        assert!(!ApiKey::unset().is_set());
        assert!(!ApiKey::default().is_set());
        assert!(!ApiKey::new(String::new()).is_set());
        assert!(!ApiKey::from_optional(None).is_set());
        assert!(!ApiKey::from_optional(Some(String::new())).is_set());

        let set = ApiKey::new("sk-test".to_owned());
        assert!(set.is_set());
        assert_eq!(set.as_deref(), Some("sk-test"));
    }

    /// 存储形态逐字未变：未设置恒为空串，已设置即明文；旧配置文件里的空串
    /// 读作未设置（向后兼容，见 ADR 0002）。
    #[test]
    fn api_key_serializes_as_a_plain_string() {
        assert_eq!(
            serde_json::to_value(ApiKey::unset()).unwrap(),
            serde_json::json!("")
        );
        assert_eq!(
            serde_json::to_value(ApiKey::new("sk-test".to_owned())).unwrap(),
            serde_json::json!("sk-test")
        );

        let legacy: ApiKey = serde_json::from_str(r#""""#).unwrap();
        assert!(!legacy.is_set(), "旧配置文件里的空串读作未设置");
    }

    /// 脱敏是类型级隔离：凭证自己知道怎么渲染，`Debug` 绝不携带明文。
    #[test]
    fn api_key_debug_output_never_contains_the_plaintext() {
        let set = ApiKey::new("sk-canary-4b7e".to_owned());
        let rendered = format!("{set:?}");
        assert!(!rendered.contains("sk-canary-4b7e"), "{rendered}");
        assert!(rendered.contains("<set>"), "{rendered}");
        assert!(format!("{:?}", ApiKey::unset()).contains("<unset>"));
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
            api_key: ApiKey::new("${KEY}".to_owned()),
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
        assert_eq!(
            projected.api_key.as_deref(),
            Some("${KEY}"),
            "api_key 原样投影、不参与插值"
        );
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
        assert_eq!(provider.api_key.as_deref(), Some("${KEY}"),);
    }

    /// 插值作用域的**哨兵**：每一个参与插值的字符串字段都填占位符，逐字段
    /// 断言被替换。
    ///
    /// 本用例刻意用**穷举字面量**构造 Provider 与 ModelEntry（不用
    /// `..Default::default()`）：新增字符串字段时它不再编译——漏掉插值会立刻
    /// 变成编译错误，而不是静默地把占位符原样投影出去。作用域的定义见
    /// ADR 0015：除 `api_key`、模型 `id` 与 header **键**外的一切字符串值；
    /// 这三处在本用例里填的是普通字面量（不是占位符），免得「不在作用域内」
    /// 与「漏掉了」两件事在断言里混作一处。
    #[test]
    fn interpolate_scope_sentinel_covers_every_string_field() {
        let provider = Provider {
            enabled: true,
            base_url: BTreeMap::from([
                (Protocol::OpenaiCompletions, "${HOST}/v1".to_owned()),
                (Protocol::AnthropicMessages, "${HOST}/messages".to_owned()),
            ]),
            selected_protocol: Some(Protocol::OpenaiCompletions),
            api_key: ApiKey::new("sk-literal".to_owned()),
            custom_header: BTreeMap::from([("X-Gateway-Key".to_owned(), "${KEY}".to_owned())]),
            models: vec![ModelEntry {
                id: "model-literal".to_owned(),
                display_name: Some("Model ${MODEL}".to_owned()),
                limit: ModelLimit {
                    context_window: Some(200_000),
                    max_input: None,
                    max_output: None,
                },
                capabilities: vec![ModelCapability::Thinking],
            }],
        };

        let projected = provider
            .interpolate(&vars(&[
                ("HOST", "api.example.com"),
                ("KEY", "sk-x"),
                ("MODEL", "gpt-4o"),
            ]))
            .unwrap();

        assert_eq!(
            projected,
            Provider {
                enabled: true,
                base_url: BTreeMap::from([
                    (Protocol::OpenaiCompletions, "api.example.com/v1".to_owned()),
                    (
                        Protocol::AnthropicMessages,
                        "api.example.com/messages".to_owned()
                    ),
                ]),
                selected_protocol: Some(Protocol::OpenaiCompletions),
                api_key: ApiKey::new("sk-literal".to_owned()),
                custom_header: BTreeMap::from([("X-Gateway-Key".to_owned(), "sk-x".to_owned())]),
                models: vec![ModelEntry {
                    id: "model-literal".to_owned(),
                    display_name: Some("Model gpt-4o".to_owned()),
                    limit: ModelLimit {
                        context_window: Some(200_000),
                        max_input: None,
                        max_output: None,
                    },
                    capabilities: vec![ModelCapability::Thinking],
                }],
            },
            "参与插值的字符串字段应逐个被替换，不在作用域内的原样保留"
        );

        // 兜底：插值结果里不应残留任何占位符。新增字段若被填了占位符却被漏掉
        // 插值，上面的期望值会与实际不符——这一句则把「漏掉了」直接点出来，
        // 不必逐字段比对才知道漏的是谁。
        let rendered = format!("{projected:?}");
        assert!(!rendered.contains("${"), "插值后不应残留占位符：{rendered}");
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

        let error = provider.interpolate(&vars(&[])).unwrap_err();

        assert_eq!(error.field, "base_url.openai-completions");
        assert!(error.reason.contains("UNDEFINED_VAR"), "{}", error.reason);
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
        assert_eq!(legacy.api_key.as_deref(), Some("sk-legacy"),);
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

        let error = provider.interpolate(&vars(&[])).unwrap_err();

        assert_eq!(error.field, "custom_header.X-Gateway-Key");
        assert!(error.reason.contains("UNDEFINED_VAR"), "{}", error.reason);
    }

    /// 模型 display_name 插值失败：字段名带数组下标（条目的字段名以自身为根，
    /// 下标路径由 `Provider::interpolate` 补齐）。
    #[test]
    fn interpolate_failure_for_model_display_name_reports_indexed_field() {
        let provider = Provider {
            models: vec![
                ModelEntry {
                    id: "plain-id".to_owned(),
                    ..ModelEntry::default()
                },
                ModelEntry {
                    id: "gpt-4o".to_owned(),
                    display_name: Some("${UNDEFINED_VAR}".to_owned()),
                    ..ModelEntry::default()
                },
            ],
            ..Provider::default()
        };

        let error = provider.interpolate(&vars(&[])).unwrap_err();

        assert_eq!(error.field, "models[1].display_name");
        assert!(error.reason.contains("UNDEFINED_VAR"), "{}", error.reason);
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
