use serde::{Deserialize, Serialize};

/// Provider 在各协议下的端点（每协议至多一个；见 ADR 0003）。
///
/// `None` 即未配置；序列化时跳过（键缺省而非空串），键缺失同样读作 `None`。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Endpoints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_completions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_messages: Option<String>,
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
    /// 凭证以明文随配置文件落盘：空串即未设置（第一期不做密钥链，
    /// ADR 0002 已修订为推迟采纳）。
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
}
