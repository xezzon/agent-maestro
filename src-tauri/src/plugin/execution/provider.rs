use serde::Serialize;
use std::collections::BTreeMap;

use super::InstantiatedPlugin;
use super::bindings::exports::maestro::plugin::plugin::{
    CustomHeader as WitCustomHeader, Endpoint as WitEndpoint, Model as WitModel,
    ModelCapability as WitModelCapability, ModelLimit as WitModelLimit, Protocol as WitProtocol,
    Provider as WitProvider,
};
use crate::provider::{ModelCapability, ModelEntry, Protocol, Provider};

impl InstantiatedPlugin {
    pub(crate) fn write_provider(
        &mut self,
        providers: &BTreeMap<String, Provider>,
    ) -> Result<(Vec<String>, Vec<SkippedProvider>), String> {
        let (wit_providers, skipped_providers) = to_wit_provider(providers);
        let handle = self.world.maestro_plugin_plugin();
        let result = handle
            .call_write_providers(&mut self.store, &wit_providers)
            .map_err(|e| format!("调用插件失败：{e}"))?;
        result
            .map(|files| (files, skipped_providers))
            .map_err(|msg| format!("插件返回错误：{msg}"))
    }
}

/// 投影结果中被跳过的 Provider 及原因（未配置任何协议端点）。
#[derive(Debug, Serialize)]
pub(crate) struct SkippedProvider {
    pub(crate) slug: String,
    pub(crate) reason: String,
}

fn to_wit_provider(
    providers: &BTreeMap<String, Provider>,
) -> (Vec<WitProvider>, Vec<SkippedProvider>) {
    let mut wit_providers = Vec::new();
    let mut skipped_provider = Vec::new();
    for (slug, provider) in providers {
        let endpoints = collect_endpoints(provider);
        if endpoints.is_empty() {
            skipped_provider.push(SkippedProvider {
                slug: slug.clone(),
                reason: "未配置任何协议端点".to_owned(),
            });
            continue;
        }
        wit_providers.push(WitProvider {
            slug: slug.to_owned(),
            endpoints,
            // 选择原样透传，可能缺失或指向未配置的协议：由插件裁定（ADR 0016）。
            selected_protocol: provider.selected_protocol.map(to_wit_protocol),
            // 空 api_key 由宿主归一化为 none（见 WIT 合同）。
            api_key: (!provider.api_key.is_empty()).then(|| provider.api_key.clone()),
            custom_header: collect_custom_headers(provider),
            models: provider.models.iter().map(to_wit_model).collect(),
        });
    }
    (wit_providers, skipped_provider)
}

/// 宿主只做映射、不做选择（ADR 0016）：按协议顺序（openai-completions 在前）
/// 收集非空端点；空串与缺省同义（未配置）。
fn collect_endpoints(provider: &Provider) -> Vec<WitEndpoint> {
    provider
        .base_url
        .iter()
        .filter(|(_, base_url)| !base_url.is_empty())
        .map(|(protocol, base_url)| WitEndpoint {
            protocol: to_wit_protocol(*protocol),
            base_url: base_url.clone(),
        })
        .collect()
}

fn to_wit_protocol(protocol: Protocol) -> WitProtocol {
    match protocol {
        Protocol::OpenaiCompletions => WitProtocol::OpenaiCompletions,
        Protocol::AnthropicMessages => WitProtocol::AnthropicMessages,
    }
}

/// 自定义 header 按名交给插件：记录层的映射（`BTreeMap`）已按键序迭代，
/// 键即 header 名。
fn collect_custom_headers(provider: &Provider) -> Vec<WitCustomHeader> {
    provider
        .custom_header
        .iter()
        .map(|(name, value)| WitCustomHeader {
            name: name.clone(),
            value: value.clone(),
        })
        .collect()
}

fn to_wit_model(model: &ModelEntry) -> WitModel {
    // 能力是集合语义：按枚举声明序（tool-use → image-in → thinking）排序并去重。
    // 记录层读入与落盘已归一，这里再归一一次，内存构造的 Provider 也是同一形状。
    // 排序在记录层枚举上做（WIT 枚举不派生 Ord），两侧变体一一对应、声明序一致。
    let mut capabilities = model.capabilities.clone();
    capabilities.sort_unstable();
    capabilities.dedup();
    WitModel {
        id: model.id.clone(),
        display_name: model.display_name.clone(),
        // 上限是声明值：未设置的子项原样传给插件（none），不填默认值。
        limit: WitModelLimit {
            context_window: model.limit.context_window,
            max_input: model.limit.max_input,
            max_output: model.limit.max_output,
        },
        capabilities: capabilities.into_iter().map(to_wit_capability).collect(),
    }
}

fn to_wit_capability(capability: ModelCapability) -> WitModelCapability {
    match capability {
        ModelCapability::ToolUse => WitModelCapability::ToolUse,
        ModelCapability::ImageIn => WitModelCapability::ImageIn,
        ModelCapability::Thinking => WitModelCapability::Thinking,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::plugin::execution::testutil::loaded_plugin;
    use crate::plugin::{build_engine, builtin};
    use crate::provider::{Endpoints, ModelLimit};

    fn provider(openai: Option<&str>, anthropic: Option<&str>) -> Provider {
        let mut base_url = Endpoints::default();
        if let Some(url) = openai {
            base_url.insert(Protocol::OpenaiCompletions, url.to_owned());
        }
        if let Some(url) = anthropic {
            base_url.insert(Protocol::AnthropicMessages, url.to_owned());
        }
        Provider {
            base_url,
            ..Provider::default()
        }
    }

    /// 宿主不再挑选：两个非空槽位都进入 `endpoints`（openai 在前），
    /// 选择原样透传给插件。
    #[test]
    fn to_wit_provider_passes_all_endpoints_and_selection_through() {
        let mut provider = provider(
            Some("https://api.example.com/v1"),
            Some("https://anthropic.example.com"),
        );
        provider.selected_protocol = Some(Protocol::AnthropicMessages);
        let providers = BTreeMap::from([("gateway".to_owned(), provider)]);

        let (mapped, skipped) = to_wit_provider(&providers);

        assert!(skipped.is_empty());
        assert_eq!(mapped.len(), 1);
        assert_eq!(mapped[0].endpoints.len(), 2, "双端点都不丢");
        assert!(matches!(
            mapped[0].endpoints[0].protocol,
            WitProtocol::OpenaiCompletions
        ));
        assert_eq!(
            mapped[0].endpoints[0].base_url,
            "https://api.example.com/v1"
        );
        assert!(matches!(
            mapped[0].endpoints[1].protocol,
            WitProtocol::AnthropicMessages
        ));
        assert_eq!(
            mapped[0].endpoints[1].base_url,
            "https://anthropic.example.com"
        );
        assert!(matches!(
            mapped[0].selected_protocol,
            Some(WitProtocol::AnthropicMessages)
        ));
    }

    #[test]
    fn to_wit_provider_skips_provider_without_any_endpoint() {
        // 空串与缺省同义：都读作未配置。
        for (openai, anthropic) in [(None, None), (Some(""), None), (None, Some(""))] {
            let providers = BTreeMap::from([("gateway".to_owned(), provider(openai, anthropic))]);

            let (mapped, skipped) = to_wit_provider(&providers);

            assert!(mapped.is_empty(), "{openai:?} / {anthropic:?}");
            assert_eq!(skipped.len(), 1);
            assert_eq!(skipped[0].slug, "gateway");
            assert_eq!(skipped[0].reason, "未配置任何协议端点");
        }
    }

    #[test]
    fn to_wit_provider_maps_endpoint_credentials_and_models() {
        let providers = BTreeMap::from([(
            "gateway".to_owned(),
            Provider {
                enabled: true,
                base_url: BTreeMap::from([(
                    Protocol::AnthropicMessages,
                    "https://anthropic.example.com".to_owned(),
                )]),
                api_key: "sk-plain".to_owned(),
                models: vec![
                    ModelEntry {
                        id: "claude-sonnet".to_owned(),
                        display_name: Some("Sonnet".to_owned()),
                        ..ModelEntry::default()
                    },
                    ModelEntry {
                        id: "claude-haiku".to_owned(),
                        display_name: None,
                        ..ModelEntry::default()
                    },
                ],
                ..Provider::default()
            },
        )]);

        let (mapped, _) = to_wit_provider(&providers);

        assert_eq!(mapped.len(), 1);
        assert_eq!(mapped[0].slug, "gateway");
        assert!(mapped[0].selected_protocol.is_none());
        assert_eq!(mapped[0].endpoints.len(), 1);
        assert!(matches!(
            mapped[0].endpoints[0].protocol,
            WitProtocol::AnthropicMessages
        ));
        assert_eq!(
            mapped[0].endpoints[0].base_url,
            "https://anthropic.example.com"
        );
        assert_eq!(mapped[0].api_key.as_deref(), Some("sk-plain"));
        assert_eq!(mapped[0].models.len(), 2);
        assert_eq!(mapped[0].models[0].id, "claude-sonnet");
        assert_eq!(mapped[0].models[0].display_name.as_deref(), Some("Sonnet"));
        assert_eq!(mapped[0].models[1].display_name, None);
    }

    /// 自定义 header 按名（字节序）交给插件；值原样映射。
    #[test]
    fn to_wit_provider_maps_custom_headers_sorted_by_name() {
        let providers = BTreeMap::from([(
            "gateway".to_owned(),
            Provider {
                custom_header: BTreeMap::from([
                    ("X-Zeta".to_owned(), "z".to_owned()),
                    ("anthropic-version".to_owned(), "2023-06-01".to_owned()),
                    ("X-Alpha".to_owned(), "a".to_owned()),
                ]),
                ..provider(Some("https://api.example.com/v1"), None)
            },
        )]);

        let (mapped, _) = to_wit_provider(&providers);

        let headers: Vec<(&str, &str)> = mapped[0]
            .custom_header
            .iter()
            .map(|header| (header.name.as_str(), header.value.as_str()))
            .collect();
        assert_eq!(
            headers,
            vec![
                ("X-Alpha", "a"),
                ("X-Zeta", "z"),
                ("anthropic-version", "2023-06-01"),
            ]
        );

        let (unset, _) = to_wit_provider(&BTreeMap::from([(
            "gateway".to_owned(),
            provider(Some("https://api.example.com/v1"), None),
        )]));
        assert!(unset[0].custom_header.is_empty(), "未设置即空列表");
    }

    /// 上限三项原样映射：未设置的子项交给插件的是 none，不填默认值。
    #[test]
    fn to_wit_model_maps_limit_subfields_and_passes_unset_as_none() {
        let mapped = to_wit_model(&ModelEntry {
            limit: ModelLimit {
                context_window: Some(200_000),
                max_input: Some(180_000),
                max_output: Some(8_192),
            },
            ..ModelEntry::default()
        });

        assert_eq!(mapped.limit.context_window, Some(200_000));
        assert_eq!(mapped.limit.max_input, Some(180_000));
        assert_eq!(mapped.limit.max_output, Some(8_192));

        let unset = to_wit_model(&ModelEntry::default());

        assert_eq!(unset.limit.context_window, None);
        assert_eq!(unset.limit.max_input, None);
        assert_eq!(unset.limit.max_output, None);
    }

    /// 能力按枚举声明序（tool-use → image-in → thinking）排序并去重。
    #[test]
    fn to_wit_model_sorts_and_deduplicates_capabilities() {
        let mapped = to_wit_model(&ModelEntry {
            capabilities: vec![
                ModelCapability::Thinking,
                ModelCapability::ToolUse,
                ModelCapability::ImageIn,
                ModelCapability::Thinking,
            ],
            ..ModelEntry::default()
        });

        assert_eq!(
            mapped.capabilities,
            vec![
                WitModelCapability::ToolUse,
                WitModelCapability::ImageIn,
                WitModelCapability::Thinking
            ]
        );
    }

    fn write_and_read(
        root: &std::path::Path,
        providers: &BTreeMap<String, Provider>,
    ) -> serde_json::Value {
        let (plugin, config_dir) = loaded_plugin(root, builtin::PI_WASM);
        let mut plugin = plugin.instantiate_component(&build_engine(), None).unwrap();

        let (files, _) = plugin.write_provider(providers).unwrap();

        assert_eq!(files, vec!["agent/models.json"]);
        serde_json::from_str(
            &fs::read_to_string(config_dir.join("agent").join("models.json")).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn write_provider_projects_into_the_preopened_config_dir() {
        let root = tempfile::tempdir().unwrap();
        let providers = BTreeMap::from([(
            "gateway".to_owned(),
            Provider {
                api_key: "sk-plain".to_owned(),
                ..provider(Some("https://api.example.com/v1"), None)
            },
        )]);

        let written = write_and_read(root.path(), &providers);

        assert_eq!(written["providers"]["gateway"]["api"], "openai-completions");
        assert_eq!(
            written["providers"]["gateway"]["baseUrl"],
            "https://api.example.com/v1"
        );
        assert_eq!(written["providers"]["gateway"]["apiKey"], "sk-plain");
    }

    /// 双端点按所选协议投影（issue #93）：未选中的端点不覆盖选择。
    #[test]
    fn write_provider_projects_the_selected_protocol() {
        let root = tempfile::tempdir().unwrap();
        let providers = BTreeMap::from([(
            "gateway".to_owned(),
            Provider {
                selected_protocol: Some(Protocol::AnthropicMessages),
                ..provider(
                    Some("https://api.example.com/v1"),
                    Some("https://anthropic.example.com"),
                )
            },
        )]);

        let written = write_and_read(root.path(), &providers);

        assert_eq!(written["providers"]["gateway"]["api"], "anthropic-messages");
        assert_eq!(
            written["providers"]["gateway"]["baseUrl"],
            "https://anthropic.example.com"
        );
    }

    /// 选择缺失（旧配置）时插件按兜底规则取唯一/首选端点：
    /// 双端点无选择 → openai-completions 在前。
    #[test]
    fn write_provider_falls_back_when_selection_is_missing() {
        let root = tempfile::tempdir().unwrap();
        let providers = BTreeMap::from([(
            "gateway".to_owned(),
            provider(
                Some("https://api.example.com/v1"),
                Some("https://anthropic.example.com"),
            ),
        )]);

        let written = write_and_read(root.path(), &providers);

        assert_eq!(written["providers"]["gateway"]["api"], "openai-completions");
        assert_eq!(
            written["providers"]["gateway"]["baseUrl"],
            "https://api.example.com/v1"
        );
    }

    /// 选择指向未配置的协议时失效，退回到唯一配置的端点。
    #[test]
    fn write_provider_falls_back_when_selection_points_to_an_absent_endpoint() {
        let root = tempfile::tempdir().unwrap();
        let providers = BTreeMap::from([(
            "gateway".to_owned(),
            Provider {
                // 只配了 anthropic，选择却指向 openai。
                selected_protocol: Some(Protocol::OpenaiCompletions),
                ..provider(None, Some("https://anthropic.example.com"))
            },
        )]);

        let written = write_and_read(root.path(), &providers);

        assert_eq!(written["providers"]["gateway"]["api"], "anthropic-messages");
        assert_eq!(
            written["providers"]["gateway"]["baseUrl"],
            "https://anthropic.example.com"
        );
    }

    /// 新字段随合同交给插件时，真实内置插件的既有投影不受扰动
    /// （新字段的投影落点在 #110/#111）。
    #[test]
    fn write_provider_keeps_existing_projection_with_new_fields_present() {
        let root = tempfile::tempdir().unwrap();
        let providers = BTreeMap::from([(
            "gateway".to_owned(),
            Provider {
                api_key: "sk-plain".to_owned(),
                custom_header: BTreeMap::from([(
                    "X-Gateway-Key".to_owned(),
                    "gw-plain".to_owned(),
                )]),
                models: vec![ModelEntry {
                    id: "gpt-4o".to_owned(),
                    display_name: Some("GPT-4o".to_owned()),
                    limit: ModelLimit {
                        context_window: Some(128_000),
                        max_input: None,
                        max_output: Some(16_384),
                    },
                    capabilities: vec![ModelCapability::ToolUse, ModelCapability::ImageIn],
                }],
                ..provider(Some("https://api.example.com/v1"), None)
            },
        )]);

        let written = write_and_read(root.path(), &providers);

        assert_eq!(written["providers"]["gateway"]["api"], "openai-completions");
        assert_eq!(
            written["providers"]["gateway"]["baseUrl"],
            "https://api.example.com/v1"
        );
        assert_eq!(written["providers"]["gateway"]["apiKey"], "sk-plain");
        assert_eq!(written["providers"]["gateway"]["models"][0]["id"], "gpt-4o");
        assert_eq!(
            written["providers"]["gateway"]["models"][0]["name"],
            "GPT-4o"
        );
    }
}
