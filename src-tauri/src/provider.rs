use serde::{Deserialize, Serialize};

/// Provider 与 LLM API 对话所用的线协议（见 ADR 0003）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Protocol {
    OpenaiCompletions,
    AnthropicMessages,
}

/// 读取侧兼容：显式空串与键缺失同样视为未配置（ADR 0003）。
fn empty_as_none<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.filter(|url| !url.is_empty()))
}

/// Provider 在各协议下的端点（每协议至多一个；见 ADR 0003）。
///
/// `None` 即未配置；序列化时跳过（键缺省而非空串），读取时键缺失与显式空串同样视为未配置。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Endpoints {
    #[serde(
        default,
        deserialize_with = "empty_as_none",
        skip_serializing_if = "Option::is_none"
    )]
    pub openai_completions: Option<String>,
    #[serde(
        default,
        deserialize_with = "empty_as_none",
        skip_serializing_if = "Option::is_none"
    )]
    pub anthropic_messages: Option<String>,
}

impl Endpoints {
    /// 设置指定协议槽位的端点（未选择的槽位不受影响）。
    pub fn set(&mut self, protocol: Protocol, base_url: &str) {
        let url = Some(base_url.to_owned());
        match protocol {
            Protocol::OpenaiCompletions => self.openai_completions = url,
            Protocol::AnthropicMessages => self.anthropic_messages = url,
        }
    }

    /// 构造仅配置单个协议端点的端点集。
    pub fn for_protocol(protocol: Protocol, base_url: &str) -> Self {
        let mut endpoints = Self::default();
        endpoints.set(protocol, base_url);
        endpoints
    }
}

/// Provider 下跨协议共享的一个模型条目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelEntry {
    #[serde(default)]
    pub id: String,
    /// 无显示名时为 `None`，序列化为 `null`，界面回退显示 id。
    #[serde(default)]
    pub display_name: Option<String>,
}

/// 一条 LLM API 接入；以 slug 为 key 存于 providers 之下（见 CONTEXT.md）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provider {
    #[serde(default)]
    pub base_url: Endpoints,
    /// 空串表示未设置凭证；否则为系统密钥链的 `secret://` 引用（见 ADR 0002）。
    #[serde(default)]
    pub api_key: String,
    /// 保序数组：模型 ID 不做字符集限制，且同一 Provider 内不重复（大小写敏感）。
    #[serde(default)]
    pub models: Vec<ModelEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_serializes_as_kebab_case() {
        assert_eq!(
            serde_json::to_value(Protocol::OpenaiCompletions).unwrap(),
            "openai-completions"
        );
        assert_eq!(
            serde_json::to_value(Protocol::AnthropicMessages).unwrap(),
            "anthropic-messages"
        );
        let parsed: Protocol = serde_json::from_value("openai-completions".into()).unwrap();
        assert_eq!(parsed, Protocol::OpenaiCompletions);
    }

    #[test]
    fn endpoints_for_protocol_fills_only_the_chosen_slot() {
        let endpoints =
            Endpoints::for_protocol(Protocol::AnthropicMessages, "http://127.0.0.1:8080");

        assert_eq!(
            endpoints.anthropic_messages,
            Some("http://127.0.0.1:8080".to_owned())
        );
        assert_eq!(endpoints.openai_completions, None);
    }

    #[test]
    fn endpoints_serialization_omits_unconfigured_slots() {
        let endpoints =
            Endpoints::for_protocol(Protocol::OpenaiCompletions, "http://localhost:11434/v1");

        let value = serde_json::to_value(&endpoints).unwrap();

        assert_eq!(
            value,
            serde_json::json!({ "openai-completions": "http://localhost:11434/v1" })
        );
    }

    #[test]
    fn endpoints_read_tolerates_explicit_empty_slots() {
        let parsed: Endpoints =
            serde_json::from_str(r#"{"openai-completions":"","anthropic-messages":""}"#).unwrap();

        assert_eq!(parsed, Endpoints::default());
    }

    #[test]
    fn endpoints_renormalize_explicit_empty_slots_on_round_trip() {
        let parsed: Endpoints = serde_json::from_str(r#"{"openai-completions":""}"#).unwrap();

        assert_eq!(parsed.openai_completions, None);
        assert_eq!(
            serde_json::to_value(&parsed).unwrap(),
            serde_json::json!({})
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
    fn provider_round_trips_dual_endpoints_secret_reference_and_models() {
        let provider = Provider {
            base_url: Endpoints {
                openai_completions: Some("https://api.example.com/v1".to_owned()),
                anthropic_messages: Some("https://anthropic.example.com/v1".to_owned()),
            },
            api_key: "secret://io.github.xezzon.agent-maestro/provider/openrouter/api_key"
                .to_owned(),
            models: vec![ModelEntry {
                id: "gpt-4o".to_owned(),
                display_name: Some("GPT-4o".to_owned()),
            }],
        };

        let parsed: Provider =
            serde_json::from_str(&serde_json::to_string(&provider).unwrap()).unwrap();

        assert_eq!(parsed, provider);
    }
}
