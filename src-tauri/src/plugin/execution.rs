mod bindings {
    wasmtime::component::bindgen!({
        path: "../crates/maestro-plugin-sdk/wit",
        world: "plugin-world",
    });
}

use bindings::exports::maestro::plugin::plugin::Protocol as WitProtocol;
/// WIT 合同 v1 的类型化绑定（宿主侧）。
use bindings::exports::maestro::plugin::plugin::{Model as WitModel, Provider as WitProvider};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use wasmtime::{
    Engine, StoreLimits, StoreLimitsBuilder,
    component::{Component, Linker},
};
use wasmtime_wasi::{FsPerms, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView, p2};

use super::LoadedPlugin;
use crate::provider::{ModelEntry, Provider};

/// 单次插件调用的执行预算（fuel）：投影任务的量级远小于该值，
/// 死循环或超量计算的插件会被中断并走失败路径，不会卡死投影。
const FUEL_BUDGET: u64 = 1 << 30;

/// 资源限制（防止恶意 memory.grow / table.grow 风暴，CWE-770）：
/// - 线性内存 64 MiB：覆盖一份 JSON 投影产物的体量上限。
/// - 表元素 1024：覆盖组件模型对 funcref 的一般需求。
/// - 单个组件典型会派生 2~4 个内部 core instance（host + component + 内嵌模块），
///   留出 8 以容纳更深的嵌套同时仍对实例数封顶。
/// - 单个组件：内存数 1（典型配置）；再小会与现有合法组件冲突。
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
const TABLE_ELEMENTS_LIMIT: usize = 1024;
const MEMORY_COUNT_LIMIT: usize = 1;
const INSTANCE_LIMIT: usize = 8;

/// 构建资源限制。
fn build_store_limits() -> StoreLimits {
    StoreLimitsBuilder::new()
        .memory_size(MEMORY_LIMIT)
        .table_elements(TABLE_ELEMENTS_LIMIT)
        .memories(MEMORY_COUNT_LIMIT)
        .instances(INSTANCE_LIMIT)
        .build()
}

/// WASI 宿主状态：唯一被授权的写入面是预开放的插件配置目录。
struct HostState {
    table: ResourceTable,
    ctx: WasiCtx,
    /// 资源限制：注册到 wasmtime Store，约束 wasm 线性内存、表、实例增长。
    limits: StoreLimits,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

impl HostState {
    /// 预开放指定配置目录为写入面，配套资源限制；用于实投影阶段。
    fn new(tool_dir: &Path) -> Result<Self, String> {
        let mut builder = WasiCtxBuilder::new();
        builder
            .preopened_dir(tool_dir, "/", FsPerms::ReadWrite)
            .map_err(|e| format!("无法开放插件配置目录：{e}"))?;
        Ok(Self {
            table: ResourceTable::new(),
            ctx: builder.build(),
            limits: build_store_limits(),
        })
    }
}

/// 已实例化的插件：实例化验证与实际调用共用同一管线。
pub struct InstantiatedPlugin {
    store: wasmtime::Store<HostState>,
    world: bindings::PluginWorld,
}

impl LoadedPlugin {
    /// 链接期校验：组件字节反序列化、导入/导出类型匹配，不触发组件 init、
    /// 不预开放真实配置目录。这是安装期使用的入口（见 ADR 0006：fail-fast）。
    pub fn validate(&self, engine: &Engine) -> Result<(), String> {
        let component =
            Component::new(engine, &self.wasm).map_err(|e| format!("不是有效的 WASM 组件：{e}"))?;
        let mut linker: Linker<HostState> = Linker::new(engine);
        p2::add_to_linker_sync(&mut linker)
            .map_err(|e| format!("初始化 WASI 宿主环境失败：{e}"))?;
        // 组件模型的 instantiate_pre 不接收 Store：返回 Pre<()> 表示类型检查通过，
        // 不触发组件 init、不调用 host 函数。这正是安装期校验所需的最小集合。
        linker
            .instantiate_pre(&component)
            .map_err(|e| format!("插件接口不兼容：{e}"))?;
        log::debug!("plugin validated: id={}", self.manifest.id);
        Ok(())
    }

    /// 完整实例化：用于实投影（write_providers）。会预开放真实配置目录、
    /// 注册资源限制（防止恶意 memory.grow / table.grow 风暴，CWE-770）。
    pub fn instantiate_component(&self, engine: &Engine) -> Result<InstantiatedPlugin, String> {
        let component =
            Component::new(engine, &self.wasm).map_err(|e| format!("不是有效的 WASM 组件：{e}"))?;
        let tool_path = PathBuf::from(self.manifest.config_dir.as_str());
        let mut store = wasmtime::Store::new(engine, HostState::new(&tool_path)?);
        store.limiter(|s| &mut s.limits);
        store
            .set_fuel(FUEL_BUDGET)
            .map_err(|e| format!("设置插件执行预算失败：{e}"))?;
        let mut linker = Linker::new(engine);
        p2::add_to_linker_sync(&mut linker)
            .map_err(|e| format!("初始化 WASI 宿主环境失败：{e}"))?;
        let world = bindings::PluginWorld::instantiate(&mut store, &component, &linker)
            .map_err(|e| format!("插件接口不兼容：{e}"))?;
        Ok(InstantiatedPlugin { store, world })
    }
}

impl InstantiatedPlugin {
    pub fn write_provider(
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
pub struct SkippedProvider {
    pub slug: String,
    pub reason: String,
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
    use crate::{
        plugin::{PlacedPlugin, build_engine, builtin, testutil},
        provider::Endpoints,
    };

    fn provider(openai: Option<&str>, anthropic: Option<&str>) -> Provider {
        Provider {
            base_url: Endpoints {
                openai_completions: openai.map(str::to_owned),
                anthropic_messages: anthropic.map(str::to_owned),
            },
            ..Provider::default()
        }
    }

    /// 已装载的插件：宿主装载时先把 manifest 的 `config_dir` 展开为绝对路径，
    /// 这里直接用临时目录充当该路径。
    fn loaded_plugin(config_dir: &Path, wasm: &[u8]) -> LoadedPlugin {
        let manifest = testutil::manifest_json(
            "pi",
            "Pi",
            "pi",
            &config_dir.display().to_string(),
            "plugin.wasm",
        );
        PlacedPlugin::new(&config_dir.join("placed"), manifest, wasm)
            .load()
            .unwrap()
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
    fn instantiate_rejects_bytes_that_are_not_a_component() {
        let config_dir = tempfile::tempdir().unwrap();

        let Err(err) = loaded_plugin(config_dir.path(), b"not a wasm component")
            .instantiate_component(&build_engine())
        else {
            panic!("非组件字节不应通过实例化校验");
        };

        assert!(err.contains("不是有效的 WASM 组件"), "{err}");
    }

    #[test]
    fn validate_rejects_bytes_that_are_not_a_component() {
        let config_dir = tempfile::tempdir().unwrap();

        let Err(err) =
            loaded_plugin(config_dir.path(), b"not a wasm component").validate(&build_engine())
        else {
            panic!("非组件字节不应通过 validate");
        };

        assert!(err.contains("不是有效的 WASM 组件"), "{err}");
    }

    #[test]
    fn validate_accepts_a_valid_component() {
        let config_dir = tempfile::tempdir().unwrap();

        loaded_plugin(config_dir.path(), builtin::PI_WASM)
            .validate(&build_engine())
            .expect("内置 pi 应通过 validate");
    }

    /// 关键安全约束：链接期校验不触发组件 init、不预开放真实配置目录。
    /// 若实现回退到完整实例化，下面的标记文件会被 init 代码读写。
    #[test]
    fn validate_does_not_touch_the_real_config_dir() {
        let config_dir = tempfile::tempdir().unwrap();
        let marker = config_dir.path().join("marker.txt");
        let original = "original-content";
        fs::write(&marker, original).unwrap();

        loaded_plugin(config_dir.path(), builtin::PI_WASM)
            .validate(&build_engine())
            .expect("validate 应通过");

        assert_eq!(
            fs::read_to_string(&marker).unwrap(),
            original,
            "validate 不得触碰真实配置目录"
        );
    }

    #[test]
    fn write_provider_projects_into_the_preopened_config_dir() {
        let config_dir = tempfile::tempdir().unwrap();
        let providers = BTreeMap::from([(
            "gateway".to_owned(),
            Provider {
                api_key: "sk-plain".to_owned(),
                ..provider(Some("https://api.example.com/v1"), None)
            },
        )]);
        let mut plugin = loaded_plugin(config_dir.path(), builtin::PI_WASM)
            .instantiate_component(&build_engine())
            .unwrap();

        let (files, _) = plugin.write_provider(&providers).unwrap();

        assert_eq!(files, vec!["agent/models.json"]);
        let written: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(config_dir.path().join("agent").join("models.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(written["providers"]["gateway"]["api"], "openai-completions");
        assert_eq!(
            written["providers"]["gateway"]["baseUrl"],
            "https://api.example.com/v1"
        );
        assert_eq!(written["providers"]["gateway"]["apiKey"], "sk-plain");
    }

    #[test]
    fn wasm_call_is_interrupted_when_fuel_exhausted() {
        let config_dir = tempfile::tempdir().unwrap();
        let mut plugin = loaded_plugin(config_dir.path(), builtin::PI_WASM)
            .instantiate_component(&build_engine())
            .unwrap();

        // 预算归零：第一条 guest 指令即触发 fuel 耗尽中断，调用转错误路径。
        plugin.store.set_fuel(0).unwrap();
        let handle = plugin.world.maestro_plugin_plugin();
        let err = handle
            .call_write_providers(&mut plugin.store, &[])
            .expect_err("fuel 耗尽应中断 wasm 调用");
        assert!(format!("{err:#}").contains("fuel"), "实际错误：{err:#}");
    }
}
