use serde::Serialize;
use std::collections::BTreeMap;

use super::InstantiatedPlugin;
use super::bindings::exports::maestro::plugin::plugin::{
    Model as WitModel, Protocol as WitProtocol, Provider as WitProvider,
};
use crate::provider::{ModelEntry, Provider};

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

/// 投影结果中被跳过的 Provider 及原因（协议槽位为零或两个非空）。
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
        match select_endpoint(provider) {
            Ok((protocol, base_url)) => {
                wit_providers.push(WitProvider {
                    slug: slug.to_owned(),
                    protocol,
                    base_url,
                    // 空 api_key 由宿主归一化为 none（见 WIT 合同）。
                    api_key: (!provider.api_key.is_empty()).then(|| provider.api_key.clone()),
                    models: provider.models.iter().map(to_wit_model).collect(),
                });
            }
            Err(reason) => {
                skipped_provider.push(SkippedProvider {
                    slug: slug.clone(),
                    reason,
                });
            }
        }
    }
    (wit_providers, skipped_provider)
}

fn to_wit_model(model: &ModelEntry) -> WitModel {
    WitModel {
        id: model.id.clone(),
        display_name: model.display_name.clone(),
    }
}

/// 宿主侧挑选规则：取唯一非空的协议槽位；零个或两个非空 → 跳过并报告原因。
fn select_endpoint(provider: &Provider) -> Result<(WitProtocol, String), String> {
    let openai = provider
        .base_url
        .openai_completions
        .as_deref()
        .filter(|url| !url.is_empty());
    let anthropic = provider
        .base_url
        .anthropic_messages
        .as_deref()
        .filter(|url| !url.is_empty());
    match (openai, anthropic) {
        (Some(url), None) => Ok((WitProtocol::OpenaiCompletions, url.to_owned())),
        (None, Some(url)) => Ok((WitProtocol::AnthropicMessages, url.to_owned())),
        (None, None) => Err("未配置任何协议端点".to_owned()),
        (Some(_), Some(_)) => Err("同时配置了两种协议端点，暂无法确定投影端点".to_owned()),
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
        Provider {
            base_url: Endpoints {
                openai_completions: openai.map(str::to_owned),
                anthropic_messages: anthropic.map(str::to_owned),
            },
            ..Provider::default()
        }
    }

    #[test]
    fn select_endpoint_takes_the_only_configured_slot() {
        let (protocol, base_url) =
            select_endpoint(&provider(Some("https://api.example.com/v1"), None)).unwrap();
        assert!(matches!(protocol, WitProtocol::OpenaiCompletions));
        assert_eq!(base_url, "https://api.example.com/v1");

        let (protocol, base_url) =
            select_endpoint(&provider(None, Some("https://anthropic.example.com"))).unwrap();
        assert!(matches!(protocol, WitProtocol::AnthropicMessages));
        assert_eq!(base_url, "https://anthropic.example.com");
    }

    #[test]
    fn select_endpoint_reports_missing_or_ambiguous_slots() {
        // 空串与缺省同义：都读作未配置。
        for (openai, anthropic) in [(None, None), (Some(""), None), (None, Some(""))] {
            assert_eq!(
                select_endpoint(&provider(openai, anthropic)).unwrap_err(),
                "未配置任何协议端点",
                "{openai:?} / {anthropic:?}"
            );
        }
        assert_eq!(
            select_endpoint(&provider(
                Some("https://a.example.com"),
                Some("https://b.example.com")
            ))
            .unwrap_err(),
            "同时配置了两种协议端点，暂无法确定投影端点"
        );
    }

    #[test]
    fn to_wit_provider_maps_endpoint_credentials_and_models() {
        let providers = BTreeMap::from([(
            "gateway".to_owned(),
            Provider {
                enabled: true,
                base_url: Endpoints {
                    anthropic_messages: Some("https://anthropic.example.com".to_owned()),
                    ..Endpoints::default()
                },
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
            },
        )]);

        let (mapped, _) = to_wit_provider(&providers);

        assert_eq!(mapped.len(), 1);
        assert_eq!(mapped[0].slug, "gateway");
        assert!(matches!(mapped[0].protocol, WitProtocol::AnthropicMessages));
        assert_eq!(mapped[0].base_url, "https://anthropic.example.com");
        assert_eq!(mapped[0].api_key.as_deref(), Some("sk-plain"));
        assert_eq!(mapped[0].models.len(), 2);
        assert_eq!(mapped[0].models[0].id, "claude-sonnet");
        assert_eq!(mapped[0].models[0].display_name.as_deref(), Some("Sonnet"));
        assert_eq!(mapped[0].models[1].display_name, None);
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
        let (plugin, config_dir) = loaded_plugin(root.path(), builtin::PI_WASM);
        let mut plugin = plugin.instantiate_component(&build_engine()).unwrap();

        let (files, _) = plugin.write_provider(&providers).unwrap();

        assert_eq!(files, vec!["agent/models.json"]);
        let written: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(config_dir.join("agent").join("models.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(written["providers"]["gateway"]["api"], "openai-completions");
        assert_eq!(
            written["providers"]["gateway"]["baseUrl"],
            "https://api.example.com/v1"
        );
        assert_eq!(written["providers"]["gateway"]["apiKey"], "sk-plain");
    }
}
