use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::interpolate::{interpolate_slot, interpolate_value};

/// Provider 在各协议下的端点（每协议至多一个；见 ADR 0003）。
///
/// `None` 即未配置；序列化时跳过（键缺省而非空串），键缺失同样读作 `None`。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct Endpoints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) openai_completions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) anthropic_messages: Option<String>,
}

/// Provider 的线协议，与 WIT 合同的 `protocol` 一一对应；序列化为 kebab-case，
/// 与 [`Endpoints`] 的槽位键同名（`openai-completions` / `anthropic-messages`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Protocol {
    OpenaiCompletions,
    AnthropicMessages,
}

/// Provider 下跨协议共享的一个模型条目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ModelEntry {
    #[serde(default)]
    pub(crate) id: String,
    /// 无显示名时为 `None`，序列化为 `null`，界面回退显示 id。
    #[serde(default)]
    pub(crate) display_name: Option<String>,
}

/// 一条 LLM API 接入；以 slug 为 key 存于 providers 之下（见 CONTEXT.md）。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Provider {
    /// 是否参与投影：默认启用，旧配置缺该字段同样视为启用（向后兼容）。
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
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
            models: Vec::new(),
        }
    }
}

/// 手工实现 Debug：api_key 渲染为 `<set>`/`<unset>`，绝不携带明文。
/// `Config` 的派生 `Debug` 内层调用它，因此嵌套在 Config 中的 Provider 同获保护
/// （ADR 0008：凭证绝不落盘进日志）。
impl std::fmt::Debug for Provider {
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
    /// 本方法只负责穷举自身的字符串字段。
    ///
    /// 作用域＝除 `api_key` 与模型 `id` 外的一切字符串值（ADR 0015）：字符串
    /// 字段穷举构造、不带
    /// `..Default`，给结构体新增字段时在此处编译失败，提示把新字段纳入插值。
    /// `enabled` 与 `selected_protocol` 是状态/枚举而非字符串值，原样透传、
    /// 不参与插值。
    pub(crate) fn interpolate(
        &self,
        variables: &BTreeMap<String, String>,
    ) -> Result<Provider, FieldError> {
        let field = "base_url.openai-completions";
        let openai_completions =
            interpolate_slot(self.base_url.openai_completions.as_deref(), variables)
                .map_err(|reason| (field.to_owned(), reason))?;
        let field = "base_url.anthropic-messages";
        let anthropic_messages =
            interpolate_slot(self.base_url.anthropic_messages.as_deref(), variables)
                .map_err(|reason| (field.to_owned(), reason))?;
        // api_key 原样保留，不参与插值。
        let api_key = self.api_key.clone();
        let mut models = Vec::with_capacity(self.models.len());
        for (index, model) in self.models.iter().enumerate() {
            let field = format!("models[{index}].display_name");
            let display_name = model
                .display_name
                .as_deref()
                .map(|name| interpolate_value(name, variables))
                .transpose()
                .map_err(|reason| (field, reason))?;
            // 模型 ID 是标识符：原样保留，不参与插值。
            models.push(ModelEntry {
                id: model.id.clone(),
                display_name,
            });
        }
        Ok(Provider {
            // enabled 是状态而非配置值：原样透传，不参与插值。
            enabled: self.enabled,
            base_url: Endpoints {
                openai_completions,
                anthropic_messages,
            },
            // selected_protocol 是枚举选择而非字符串值：原样透传，不参与插值。
            selected_protocol: self.selected_protocol,
            api_key,
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

        assert_eq!(parsed.openai_completions, Some(String::new()));
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
            parsed.base_url.anthropic_messages,
            Some("http://127.0.0.1:8080".to_owned())
        );
        assert_eq!(parsed.base_url.openai_completions, None);
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
        };

        let text = serde_json::to_string(&entry).unwrap();

        assert_eq!(text, r#"{"id":"deepseek-chat","display_name":null}"#);
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
                },
                ModelEntry {
                    id: "a-model".to_owned(),
                    display_name: Some("A Model".to_owned()),
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
            base_url: Endpoints {
                openai_completions: Some("https://api.example.com/v1".to_owned()),
                anthropic_messages: Some("https://anthropic.example.com/v1".to_owned()),
            },
            api_key: "sk-test".to_owned(),
            models: vec![ModelEntry {
                id: "gpt-4o".to_owned(),
                display_name: Some("GPT-4o".to_owned()),
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
            base_url: Endpoints {
                openai_completions: Some("https://a.example.com/v1".to_owned()),
                anthropic_messages: Some("https://b.example.com".to_owned()),
            },
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

    /// 插值（`Provider::interpolate`）输出全部字面值副本：除模型 ID 与 api_key
    /// （原样保留）外逐字段替换，原值不动。
    #[test]
    fn interpolate_replaces_strings_field_by_field_and_keeps_the_original() {
        let provider = Provider {
            enabled: true,
            base_url: Endpoints {
                openai_completions: Some("https://${HOST}/v1".to_owned()),
                anthropic_messages: None,
            },
            selected_protocol: Some(Protocol::OpenaiCompletions),
            api_key: "${KEY}".to_owned(),
            models: vec![
                ModelEntry {
                    id: "${MODEL}".to_owned(),
                    display_name: Some("Model ${MODEL}".to_owned()),
                },
                ModelEntry {
                    id: "plain-id".to_owned(),
                    display_name: None,
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
            projected.base_url.openai_completions.as_deref(),
            Some("https://api.example.com/v1")
        );
        assert_eq!(
            projected.selected_protocol,
            Some(Protocol::OpenaiCompletions),
            "选择是枚举，原样投影、不参与插值"
        );
        assert_eq!(projected.api_key, "${KEY}", "api_key 原样投影、不参与插值");
        assert_eq!(
            projected.models[0].id, "${MODEL}",
            "模型 ID 是标识符，原样投影、不参与插值"
        );
        assert_eq!(
            projected.models[0].display_name.as_deref(),
            Some("Model gpt-4o")
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
            base_url: Endpoints {
                openai_completions: Some("${UNDEFINED_VAR}".to_owned()),
                ..Endpoints::default()
            },
            ..Provider::default()
        };

        let (field, reason) = provider.interpolate(&vars(&[])).unwrap_err();

        assert_eq!(field, "base_url.openai-completions");
        assert!(reason.contains("UNDEFINED_VAR"), "{reason}");
    }
}
