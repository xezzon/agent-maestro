//! 插件服务：从配置条目装载插件（内置字节）、维护内存注册表、
//! 并把 Provider 投影进各插件声明的配置目录（wasmtime 宿主，WASI 0.2）。
//!
//! 架构决策见 ADR 0004：宿主不代写文件，而是把 manifest 声明的 `config_dir`
//! 预开放给组件（guest 路径 `/`），插件在沙箱内经 WASI 直接落盘。

pub mod builtin;
pub mod manifest;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};
use wasmtime::{
    Engine,
    component::{Component, Linker},
};
use wasmtime_wasi::{FsPerms, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView, p2};

use crate::{provider::Provider, store::Store};

mod bindings {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "plugin-world",
    });
}

use bindings::exports::maestro::plugin::plugin::Protocol as WitProtocol;
/// WIT 合同 v1 的类型化绑定（宿主侧）。
use bindings::exports::maestro::plugin::plugin::{Model as WitModel, Provider as WitProvider};
use manifest::parse_manifest;

/// 插件条目（config.json 的 `plugins` 段，纯增量字段；见 issue #34）。
///
/// `source` 是条目唯一身份，重复添加在 store 层拒绝；第一期仅内置来源
/// （`builtin:<id>`，Git / 本地来源见 issue #36）。
/// `id` 为插件 id（内置条目在 upsert 时写入）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEntry {
    pub source: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

fn default_true() -> bool {
    true
}

/// 插件列表视图（命令返回给前端的形态）。
#[derive(Debug, Serialize)]
pub struct PluginView {
    pub source: String,
    /// 来源以内置前缀标识；内置插件可禁用、不可移除、不出现在添加流程。
    pub builtin: bool,
    pub enabled: bool,
    pub id: Option<String>,
    pub name: Option<String>,
    pub tool: Option<String>,
    /// manifest 声明的配置目录（`~` 已展开为绝对路径）。
    pub config_dir: Option<String>,
    /// `loaded` 或 `error`。
    pub status: &'static str,
    pub error: Option<String>,
}

/// 投影结果中被跳过的 Provider 及原因（协议槽位为零或两个非空）。
#[derive(Debug, Serialize)]
pub struct SkippedProvider {
    pub slug: String,
    pub reason: String,
}

/// 逐插件投影报告：状态、已写入文件、跳过的 Provider、失败原因。
#[derive(Debug, Serialize)]
pub struct PluginApplyReport {
    pub source: String,
    pub id: Option<String>,
    pub name: Option<String>,
    /// `applied` | `failed` | `skipped`（禁用或加载失败时不执行投影）。
    pub status: &'static str,
    /// 已写入文件的路径列表（相对 config_dir，由插件返回）。
    pub files: Vec<String>,
    pub skipped: Vec<SkippedProvider>,
    pub reason: Option<String>,
}

/// 成功装载的插件（metadata + 内置 wasm 字节 + 已展开的配置目录）。
#[derive(Debug, Clone)]
struct LoadedPlugin {
    id: String,
    name: String,
    tool: String,
    config_dir: PathBuf,
    wasm: &'static [u8],
}

/// 注册表条目的装载状态；装载失败进错误态并携带人类可读原因。
#[derive(Debug, Clone)]
enum PluginState {
    Loaded(LoadedPlugin),
    Error(String),
}

#[derive(Debug, Clone)]
struct RegistryEntry {
    source: String,
    builtin: bool,
    enabled: bool,
    state: PluginState,
}

/// 插件服务：内存注册表随配置/磁盘变更整体重建（`reload`）。
pub struct PluginService {
    engine: Engine,
    /// 用于展开 manifest 的 `~`；测试可替换为临时主目录。
    home: Option<PathBuf>,
    entries: Mutex<Vec<RegistryEntry>>,
}

/// WASI 宿主状态：唯一被授权的写入面是预开放的插件配置目录。
struct HostState {
    table: ResourceTable,
    ctx: WasiCtx,
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
    fn new(config_dir: &Path) -> Result<Self, String> {
        let mut builder = WasiCtxBuilder::new();
        builder
            .preopened_dir(config_dir, "/", FsPerms::ReadWrite)
            .map_err(|e| format!("无法开放插件配置目录：{e}"))?;
        Ok(Self {
            table: ResourceTable::new(),
            ctx: builder.build(),
        })
    }
}

/// 已实例化的插件：实例化验证与实际调用共用同一管线。
struct InstantiatedPlugin {
    store: wasmtime::Store<HostState>,
    world: bindings::PluginWorld,
}

/// 实例化组件（编译通过 + 导出接口匹配 + config_dir 可开放 + fuel 预算就位）；
/// 装载时用于兼容性校验，投影时用于实际调用。
fn instantiate_component(
    engine: &Engine,
    bytes: &[u8],
    config_dir: &Path,
) -> Result<InstantiatedPlugin, String> {
    let component =
        Component::new(engine, bytes).map_err(|e| format!("不是有效的 WASM 组件：{e}"))?;
    let mut store = wasmtime::Store::new(engine, HostState::new(config_dir)?);
    store
        .set_fuel(FUEL_BUDGET)
        .map_err(|e| format!("设置插件执行预算失败：{e}"))?;
    let mut linker = Linker::new(engine);
    p2::add_to_linker_sync(&mut linker).map_err(|e| format!("初始化 WASI 宿主环境失败：{e}"))?;
    let world = bindings::PluginWorld::instantiate(&mut store, &component, &linker)
        .map_err(|e| format!("插件接口不兼容：{e}"))?;
    Ok(InstantiatedPlugin { store, world })
}

/// 单次插件调用的执行预算（fuel）：投影任务的量级远小于该值，
/// 死循环或超量计算的插件会被中断并走失败路径，不会卡死投影。
const FUEL_BUDGET: u64 = 1 << 30;

/// 引擎配置：启用 fuel 计量（配合 `FUEL_BUDGET` 限制插件执行时长）。
fn build_engine() -> Engine {
    let mut config = wasmtime::Config::new();
    config.consume_fuel(true);
    Engine::new(&config).expect("failed to create wasm engine")
}

impl Default for PluginService {
    fn default() -> Self {
        Self {
            engine: build_engine(),
            home: dirs::home_dir(),
            entries: Default::default(),
        }
    }
}

impl PluginService {
    fn home(&self) -> Result<&Path, String> {
        self.home
            .as_deref()
            .ok_or_else(|| "无法确定用户主目录（HOME）".to_owned())
    }

    /// 应用启动：upsert 内置条目（离线、只读磁盘）并重建注册表。
    pub fn startup(&self, store: &Mutex<Store>) {
        let mut guard = match store.lock() {
            Ok(guard) => guard,
            Err(_) => {
                eprintln!("failed to lock store during plugin startup");
                return;
            }
        };
        if let Err(e) =
            guard.upsert_builtin_plugin(builtin::BUILTIN_PI_SOURCE, builtin::BUILTIN_PI_ID)
        {
            eprintln!("failed to upsert builtin plugin entry: {}", e.message());
        }
        self.reload(&guard);
    }

    /// 只读磁盘重建注册表，不联网（手动修复插件文件后无需重启应用）。
    pub fn reload(&self, store: &Store) {
        let entries = store
            .get()
            .map(|config| config.plugins.clone())
            .unwrap_or_default();
        let built = entries
            .into_iter()
            .map(|entry| self.build_entry(entry))
            .collect();
        *self.entries.lock().unwrap() = built;
    }

    /// 按配置条目顺序装载；第一期仅支持内置来源，其余条目进错误态（issue #36 回归）。
    fn build_entry(&self, entry: PluginEntry) -> RegistryEntry {
        let builtin = entry.source.starts_with("builtin:");
        let state = match self.load_entry(&entry) {
            Ok(loaded) => PluginState::Loaded(loaded),
            Err(reason) => PluginState::Error(reason),
        };
        RegistryEntry {
            source: entry.source,
            builtin,
            enabled: entry.enabled,
            state,
        }
    }

    fn load_entry(&self, entry: &PluginEntry) -> Result<LoadedPlugin, String> {
        if entry.source.starts_with("builtin:") {
            return self.load_builtin();
        }
        Err(format!("暂不支持的插件来源：{}", entry.source))
    }

    /// 装载管线：解析 manifest → 解析 config_dir（`~` 展开、目录不存在则先创建）→ 实例化校验。
    fn load_builtin(&self) -> Result<LoadedPlugin, String> {
        let manifest =
            parse_manifest(builtin::PI_MANIFEST_JSON).map_err(|e| format!("内置插件损坏：{e}"))?;
        let config_dir = resolve_config_dir(&manifest.config_dir, self.home()?)?;
        instantiate_component(&self.engine, builtin::PI_WASM, &config_dir)?;
        Ok(LoadedPlugin {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            tool: manifest.tool.clone(),
            config_dir,
            wasm: builtin::PI_WASM,
        })
    }

    /// 当前注册表视图。
    pub fn list(&self) -> Vec<PluginView> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .map(|entry| {
                let base = PluginView {
                    source: entry.source.clone(),
                    builtin: entry.builtin,
                    enabled: entry.enabled,
                    id: None,
                    name: None,
                    tool: None,
                    config_dir: None,
                    status: "error",
                    error: None,
                };
                match &entry.state {
                    PluginState::Loaded(p) => PluginView {
                        id: Some(p.id.clone()),
                        name: Some(p.name.clone()),
                        tool: Some(p.tool.clone()),
                        config_dir: Some(p.config_dir.display().to_string()),
                        status: "loaded",
                        ..base
                    },
                    PluginState::Error(reason) => PluginView {
                        error: Some(reason.clone()),
                        ..base
                    },
                }
            })
            .collect()
    }

    /// 启用/禁用插件条目。
    pub fn set_enabled(
        &self,
        store: &mut Store,
        source: &str,
        enabled: bool,
    ) -> Result<(), String> {
        store
            .set_plugin_enabled(source, enabled)
            .map_err(|e| e.message())?;
        self.reload(store);
        Ok(())
    }

    /// 投影：调用所有已启用且加载成功的插件；单个插件失败不影响其他插件。
    pub fn apply(&self, providers: &BTreeMap<String, Provider>) -> Vec<PluginApplyReport> {
        let entries = self.entries.lock().unwrap().clone();
        entries
            .iter()
            .map(|entry| self.apply_entry(entry, providers))
            .collect()
    }

    fn apply_entry(
        &self,
        entry: &RegistryEntry,
        providers: &BTreeMap<String, Provider>,
    ) -> PluginApplyReport {
        let mut report = PluginApplyReport {
            source: entry.source.clone(),
            id: None,
            name: None,
            status: "skipped",
            files: Vec::new(),
            skipped: Vec::new(),
            reason: None,
        };
        if !entry.enabled {
            report.reason = Some("已禁用".to_owned());
            return report;
        }
        let loaded = match &entry.state {
            PluginState::Loaded(loaded) => loaded,
            PluginState::Error(reason) => {
                report.reason = Some(format!("插件加载失败：{reason}"));
                return report;
            }
        };
        report.id = Some(loaded.id.clone());
        report.name = Some(loaded.name.clone());
        let mut wit_providers = Vec::new();
        for (slug, provider) in providers {
            match select_endpoint(provider) {
                Ok((protocol, base_url)) => {
                    wit_providers.push(to_wit_provider(slug, provider, protocol, base_url));
                }
                Err(reason) => report.skipped.push(SkippedProvider {
                    slug: slug.clone(),
                    reason,
                }),
            }
        }
        match self.call_write_providers(loaded, wit_providers) {
            Ok(files) => {
                report.status = "applied";
                report.files = files;
            }
            Err(reason) => {
                report.status = "failed";
                report.reason = Some(reason);
            }
        }
        report
    }

    fn call_write_providers(
        &self,
        loaded: &LoadedPlugin,
        providers: Vec<WitProvider>,
    ) -> Result<Vec<String>, String> {
        let mut plugin = instantiate_component(&self.engine, loaded.wasm, &loaded.config_dir)?;
        let handle = plugin.world.maestro_plugin_plugin();
        let result = handle
            .call_write_providers(&mut plugin.store, &providers)
            .map_err(|e| format!("调用插件失败：{e}"))?;
        result.map_err(|msg| format!("插件返回错误：{msg}"))
    }
}

/// 宿主侧挑选规则：取唯一非空的协议槽位；零个或两个非空 → 跳过并报告原因。
fn select_endpoint(provider: &Provider) -> Result<(EndpointProtocol, String), String> {
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
        (Some(url), None) => Ok((EndpointProtocol::OpenaiCompletions, url.to_owned())),
        (None, Some(url)) => Ok((EndpointProtocol::AnthropicMessages, url.to_owned())),
        (None, None) => Err("未配置任何协议端点".to_owned()),
        (Some(_), Some(_)) => Err("同时配置了两种协议端点，暂无法确定投影端点".to_owned()),
    }
}

enum EndpointProtocol {
    OpenaiCompletions,
    AnthropicMessages,
}

/// 映射为 WIT 合同的单协议 Provider：每条只携带一个协议的端点；
/// 空 api_key 由宿主归一化为 none。
fn to_wit_provider(
    slug: &str,
    provider: &Provider,
    protocol: EndpointProtocol,
    base_url: String,
) -> WitProvider {
    WitProvider {
        slug: slug.to_owned(),
        protocol: match protocol {
            EndpointProtocol::OpenaiCompletions => WitProtocol::OpenaiCompletions,
            EndpointProtocol::AnthropicMessages => WitProtocol::AnthropicMessages,
        },
        base_url,
        api_key: (!provider.api_key.is_empty()).then(|| provider.api_key.clone()),
        models: provider
            .models
            .iter()
            .map(|model| WitModel {
                id: model.id.clone(),
                display_name: model.display_name.clone(),
            })
            .collect(),
    }
}

/// 解析 manifest 声明的配置目录为宿主可控范围内的绝对路径。
///
/// 仅接受 `~/…` 形式（展开为主目录下的相对路径），拒绝绝对路径、`..` 上跳
/// 与空的 `~`；目录创建后规范化验证仍在主目录内，防止外置 manifest 声明
/// 任意主机目录并借 WASI 预开放获得读写权限。
fn resolve_config_dir(config_dir: &str, home: &Path) -> Result<PathBuf, String> {
    let rest = config_dir
        .strip_prefix("~/")
        .filter(|rest| !rest.is_empty())
        .ok_or_else(|| {
            format!(
                "manifest.json 不合法：config_dir「{config_dir}」必须是主目录内的相对路径（~/…）"
            )
        })?;
    let relative = Path::new(rest);
    if relative
        .components()
        .any(|c| c == std::path::Component::ParentDir)
    {
        return Err(format!(
            "manifest.json 不合法：config_dir「{config_dir}」不得包含「..」"
        ));
    }
    let dir = home.join(relative);
    fs::create_dir_all(&dir).map_err(|e| format!("创建插件配置目录失败：{e}"))?;
    let canonical_home = home
        .canonicalize()
        .map_err(|e| format!("解析主目录失败：{e}"))?;
    let canonical = dir
        .canonicalize()
        .map_err(|e| format!("解析插件配置目录失败：{e}"))?;
    if !canonical.starts_with(&canonical_home) {
        return Err(format!(
            "manifest.json 不合法：config_dir「{config_dir}」逃逸了主目录"
        ));
    }
    Ok(canonical)
}

/// 测试共享助手：临时主目录 + 独立引擎的插件服务与配置存储。
#[cfg(test)]
pub(crate) mod testutil {
    use crate::Mutex;
    use crate::store::Store;
    use std::path::{Path, PathBuf};

    use super::PluginService;

    impl PluginService {
        /// 测试专用构造器：注入任意主目录；生产代码使用 `Default`（读取真实 `~`）。
        pub fn new(home: Option<PathBuf>) -> Self {
            Self {
                engine: super::build_engine(),
                home,
                entries: Mutex::new(Vec::new()),
            }
        }
    }

    pub(crate) fn temp_home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    pub(crate) fn test_service(home: &Path) -> PluginService {
        PluginService::new(Some(home.to_owned()))
    }

    pub(crate) fn store_at(home: &Path) -> Store {
        Store::open(home.join(".maestro").join("config.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::{store_at, temp_home, test_service};
    use super::*;
    use crate::provider::{Endpoints, ModelEntry};

    fn provider_openai(url: &str, api_key: &str, models: Vec<ModelEntry>) -> Provider {
        Provider {
            base_url: Endpoints {
                openai_completions: Some(url.to_owned()),
                anthropic_messages: None,
            },
            api_key: api_key.to_owned(),
            models,
        }
    }

    fn startup_service(home: &Path) -> PluginService {
        let store = Mutex::new(store_at(home));
        let service = test_service(home);
        service.startup(&store);
        service
    }

    // ---- 装载 ----

    #[test]
    fn startup_upserts_builtin_plugin_and_registry_shows_it_loaded() {
        let home = temp_home();
        let service = startup_service(home.path());

        let views = service.list();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].source, builtin::BUILTIN_PI_SOURCE);
        assert!(views[0].builtin);
        assert!(views[0].enabled);
        assert_eq!(views[0].status, "loaded");
        assert_eq!(views[0].id.as_deref(), Some("pi"));
        assert_eq!(views[0].name.as_deref(), Some("Pi"));
        assert_eq!(views[0].tool.as_deref(), Some("pi"));
        let config_dir = views[0].config_dir.as_deref().unwrap();
        assert!(
            config_dir.ends_with(".pi"),
            "~ 已展开为主目录：{config_dir}"
        );
    }

    #[test]
    fn startup_writes_builtin_config_entry_to_disk() {
        let home = temp_home();
        let store = Mutex::new(store_at(home.path()));
        let service = test_service(home.path());
        service.startup(&store);

        // 启动 upsert 落盘；重启（重新 open）后条目仍在。
        let reopened = Store::open(home.path().join(".maestro").join("config.json"));
        let plugins = &reopened.get().unwrap().plugins;
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].source, builtin::BUILTIN_PI_SOURCE);
        assert_eq!(plugins[0].id.as_deref(), Some("pi"));
    }

    #[test]
    fn non_builtin_source_entry_is_reported_as_error_state() {
        let home = temp_home();
        // 旧版本配置可能残留 Git 来源条目：第一期不支持，进错误态而非静默忽略（Git 来源见 issue #36）。
        let path = home.path().join(".maestro").join("config.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"version":1,"providers":{},"plugins":[{"source":"https://example.com/x.git","enabled":true}]}"#,
        )
        .unwrap();
        let store = Mutex::new(Store::open(path));
        let service = test_service(home.path());
        service.startup(&store);

        let views = service.list();
        assert_eq!(views.len(), 2);
        let git = views
            .iter()
            .find(|v| v.source == "https://example.com/x.git")
            .unwrap();
        assert_eq!(git.status, "error");
        assert!(
            git.error.as_deref().unwrap().contains("暂不支持的插件来源"),
            "实际错误：{:?}",
            git.error
        );
    }

    // ---- 投影 ----

    #[test]
    fn apply_projects_providers_into_pi_models_json() {
        let home = temp_home();
        let service = startup_service(home.path());

        // 预置僵尸条目：整文件重写后不得残留。
        let pi_agent_dir = home.path().join(".pi").join("agent");
        fs::create_dir_all(&pi_agent_dir).unwrap();
        fs::write(
            pi_agent_dir.join("models.json"),
            r#"{"providers":{"zombie":{"baseUrl":"http://gone.example.com"}}}"#,
        )
        .unwrap();

        let mut store = store_at(home.path());
        store
            .create_provider(
                "gateway",
                provider_openai(
                    "https://api.example.com/v1",
                    "sk-plain",
                    vec![
                        ModelEntry {
                            id: "gpt-4o".to_owned(),
                            display_name: Some("GPT-4o".to_owned()),
                        },
                        ModelEntry {
                            id: "deepseek-chat".to_owned(),
                            display_name: None,
                        },
                    ],
                ),
            )
            .unwrap();
        store
            .create_provider(
                "escaped-dollar",
                provider_openai("https://api.example.com/v1", "$ENV_SECRET", vec![]),
            )
            .unwrap();
        store
            .create_provider(
                "escaped-bang",
                provider_openai("https://api.example.com/v1", "!cmd", vec![]),
            )
            .unwrap();
        store
            .create_provider(
                "local",
                provider_openai("http://localhost:11434/v1", "", vec![]),
            )
            .unwrap();
        store
            .create_provider(
                "anthropic-only",
                Provider {
                    base_url: Endpoints {
                        openai_completions: None,
                        anthropic_messages: Some("https://anthropic.example.com".to_owned()),
                    },
                    api_key: "sk-a".to_owned(),
                    models: vec![],
                },
            )
            .unwrap();
        store
            .create_provider(
                "dual",
                Provider {
                    base_url: Endpoints {
                        openai_completions: Some("https://a.example.com".to_owned()),
                        anthropic_messages: Some("https://b.example.com".to_owned()),
                    },
                    api_key: String::new(),
                    models: vec![],
                },
            )
            .unwrap();
        store
            .create_provider("no-endpoint", Provider::default())
            .unwrap();

        let providers = &store.get().unwrap().providers;
        let reports = service.apply(providers);

        assert_eq!(reports.len(), 1);
        let report = &reports[0];
        assert_eq!(report.status, "applied", "reason: {:?}", report.reason);
        assert_eq!(report.files, vec!["agent/models.json"]);
        let skipped: Vec<_> = report
            .skipped
            .iter()
            .map(|s| (s.slug.as_str(), s.reason.as_str()))
            .collect();
        assert_eq!(
            skipped,
            vec![
                ("dual", "同时配置了两种协议端点，暂无法确定投影端点"),
                ("no-endpoint", "未配置任何协议端点"),
            ]
        );

        let models_json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(pi_agent_dir.join("models.json")).unwrap())
                .unwrap();
        let providers_json = &models_json["providers"];

        assert!(
            providers_json.get("zombie").is_none(),
            "整文件重写不得残留僵尸条目"
        );
        assert_eq!(
            providers_json["gateway"]["baseUrl"],
            "https://api.example.com/v1"
        );
        assert_eq!(providers_json["gateway"]["api"], "openai-completions");
        assert_eq!(providers_json["gateway"]["apiKey"], "sk-plain");
        assert_eq!(
            providers_json["gateway"]["models"][0]["id"], "gpt-4o",
            "模型列表映射进 providers 条目"
        );
        assert_eq!(
            providers_json["gateway"]["models"][0]["name"], "GPT-4o",
            "display_name 映射为模型 name"
        );
        assert!(
            providers_json["gateway"]["models"][1].get("name").is_none(),
            "display_name 缺省则省略 name"
        );
        assert_eq!(
            providers_json["anthropic-only"]["api"],
            "anthropic-messages"
        );
        assert_eq!(
            providers_json["escaped-dollar"]["apiKey"], "$$ENV_SECRET",
            "以 $ 开头的明文 key 按转义规则写为 $$"
        );
        assert_eq!(providers_json["escaped-bang"]["apiKey"], "$!cmd");
        assert!(
            providers_json["local"].get("apiKey").is_none(),
            "空 api_key 省略字段（模型可见，/login 兜底）"
        );
        assert!(providers_json["local"]["baseUrl"] == "http://localhost:11434/v1");
    }

    #[test]
    fn disabled_plugin_is_skipped_in_apply() {
        let home = temp_home();
        let store = Mutex::new(store_at(home.path()));
        let service = test_service(home.path());
        service.startup(&store);
        service
            .set_enabled(
                &mut store.lock().unwrap(),
                builtin::BUILTIN_PI_SOURCE,
                false,
            )
            .unwrap();

        let reports = service.apply(&BTreeMap::new());
        assert_eq!(reports[0].status, "skipped");
        assert_eq!(reports[0].reason.as_deref(), Some("已禁用"));
    }

    #[test]
    fn wasm_call_is_interrupted_when_fuel_exhausted() {
        let home = temp_home();
        let service = test_service(home.path());
        let config_dir = home.path().join("config");
        fs::create_dir_all(&config_dir).unwrap();

        let mut plugin = instantiate_component(&service.engine, builtin::PI_WASM, &config_dir)
            .expect("内置 pi 组件应可实例化");
        // 预算归零：第一条 guest 指令即触发 fuel 耗尽中断，调用转错误路径。
        plugin.store.set_fuel(0).unwrap();
        let handle = plugin.world.maestro_plugin_plugin();
        let err = handle
            .call_write_providers(&mut plugin.store, &Vec::new())
            .expect_err("fuel 耗尽应中断 wasm 调用");
        assert!(format!("{err:#}").contains("fuel"), "实际错误：{err:#}");
    }
}
