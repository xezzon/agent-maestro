use serde::Serialize;
use std::collections::BTreeMap;

use super::InstantiatedPlugin;
use super::bindings::exports::maestro::plugin::plugin::{
    Endpoint as WitEndpoint, Model as WitModel, Protocol as WitProtocol, Provider as WitProvider,
};
use crate::provider::{ModelEntry, Protocol, Provider};

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

fn to_wit_model(model: &ModelEntry) -> WitModel {
    WitModel {
        id: model.id.clone(),
        display_name: model.display_name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::plugin::execution::testutil::loaded_plugin;
    use crate::plugin::{build_engine, builtin};
    use crate::provider::Endpoints;

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
                    },
                    ModelEntry {
                        id: "claude-haiku".to_owned(),
                        display_name: None,
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
}
