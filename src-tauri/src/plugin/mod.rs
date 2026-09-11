//! 插件服务：从配置条目装载插件（内置字节或落位目录）、维护内存注册表、
//! 并把 Provider 投影进各插件声明的配置目录（wasmtime 宿主，WASI 0.2）。
//!
//! 架构决策见 ADR 0004：宿主不代写文件，而是把 manifest 声明的 `config_dir`
//! 预开放给组件（guest 路径 `/`），插件在沙箱内经 WASI 直接落盘。
//! https 来源的下载、落位与生命周期见 ADR 0006。

pub mod builtin;
pub mod fetch;
pub mod install;
pub mod manifest;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};
use wasmtime::{
    Engine,
    component::{Component, Linker},
};
use wasmtime_wasi::{FsPerms, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView, p2};

use crate::{
    provider::Provider,
    store::{Store, StoreError},
};
use fetch::{Fetcher, HttpFetcher};
use manifest::{SourceKind, is_https_url, parse_manifest};

mod bindings {
    wasmtime::component::bindgen!({
        path: "../crates/maestro-plugin-sdk/wit",
        world: "plugin-world",
    });
}

use bindings::exports::maestro::plugin::plugin::Protocol as WitProtocol;
/// WIT 合同 v1 的类型化绑定（宿主侧）。
use bindings::exports::maestro::plugin::plugin::{Model as WitModel, Provider as WitProvider};

/// 插件条目（config.json 的 `plugins` 段，纯增量字段；见 issue #34）。
///
/// `source` 是条目唯一身份（内置 `builtin:<id>` 或指向 manifest.json 的 https URL），
/// 重复添加在 store 层拒绝；`id` 为插件 id，内置条目在 upsert 时写入，
/// https 条目在安装成功后写入——它是来源到落位目录的唯一映射（见 ADR 0006）。
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

/// 成功装载的插件（metadata + wasm 字节 + 已展开的配置目录）。
///
/// wasm 字节以 `Arc` 共享：内置插件是内嵌字节，https 插件是落位目录读入的产物。
#[derive(Debug, Clone)]
struct LoadedPlugin {
    id: String,
    name: String,
    tool: String,
    config_dir: PathBuf,
    wasm: Arc<[u8]>,
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
    /// 配置中记录的插件 id（内置条目启动时写入，https 条目安装成功后写入）；
    /// 错误态下作为「这条来源本该是哪个插件」的线索。
    id: Option<String>,
    state: PluginState,
}

/// 插件服务：内存注册表随配置/磁盘变更整体重建（`rebuild`）。
pub struct PluginService {
    engine: Engine,
    /// 用于展开 manifest 的 `~`；测试可替换为临时主目录。
    home: Option<PathBuf>,
    /// https 拉取的注入点（生产为同步 reqwest 实现，测试为替身）。
    fetcher: Arc<dyn Fetcher>,
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
        Self::with_fetcher(dirs::home_dir(), Arc::new(HttpFetcher::new()))
    }
}

impl PluginService {
    /// 注入 fetcher：生产用 https 实现，测试用替身（本设计唯一的新缝）。
    pub fn with_fetcher(home: Option<PathBuf>, fetcher: Arc<dyn Fetcher>) -> Self {
        Self {
            engine: build_engine(),
            home,
            fetcher,
            entries: Mutex::new(Vec::new()),
        }
    }

    fn home(&self) -> Result<&Path, String> {
        self.home
            .as_deref()
            .ok_or_else(|| "无法确定用户主目录（HOME）".to_owned())
    }

    /// 指定插件 id 的落位目录（`~/.maestro/plugins/<id>`）。
    fn plugin_dir(&self, id: &str) -> Result<PathBuf, String> {
        Ok(install::plugin_dir(&install::root(self.home()?), id))
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
        self.rebuild(&guard);
    }

    /// 只读磁盘重建注册表，不联网；启动与来源变更后调用。
    ///
    /// 按来源回源获取由 [`PluginService::reload_plugin`] 负责。同一插件 id 只能由一个
    /// 来源持有：后加载者进错误态并指明冲突来源。
    fn rebuild(&self, store: &Store) {
        let entries = store
            .get()
            .map(|config| config.plugins.clone())
            .unwrap_or_default();
        let mut claimed: BTreeMap<String, String> = BTreeMap::new();
        let mut registry = Vec::with_capacity(entries.len());
        for entry in entries {
            let mut built = self.build_entry(&entry);
            if let Some(id) = &entry.id {
                match claimed.get(id) {
                    Some(owner) => {
                        built.state = PluginState::Error(format!(
                            "插件 id「{id}」与来源 {owner} 冲突：请移除其中一个条目"
                        ));
                    }
                    None => {
                        claimed.insert(id.clone(), entry.source.clone());
                    }
                }
            }
            registry.push(built);
        }
        *self.entries.lock().unwrap() = registry;
    }

    /// 按配置条目顺序装载；不认识的来源形态进错误态（如旧配置残留的 Git 来源）。
    fn build_entry(&self, entry: &PluginEntry) -> RegistryEntry {
        let builtin = entry.source.starts_with(builtin::SOURCE_PREFIX);
        let state = match self.load_entry(entry) {
            Ok(loaded) => PluginState::Loaded(loaded),
            Err(reason) => PluginState::Error(reason),
        };
        RegistryEntry {
            source: entry.source.clone(),
            builtin,
            enabled: entry.enabled,
            id: entry.id.clone(),
            state,
        }
    }

    fn load_entry(&self, entry: &PluginEntry) -> Result<LoadedPlugin, String> {
        if entry.source.starts_with(builtin::SOURCE_PREFIX) {
            return self.load_builtin();
        }
        if !is_https_url(&entry.source) {
            return Err(format!("暂不支持的插件来源：{}", entry.source));
        }
        let id = entry.id.as_deref().ok_or_else(|| {
            "尚未安装成功（下载、校验或落位失败），请点「重新加载」重试".to_owned()
        })?;
        self.load_placed(id)
    }

    /// 落位插件的装载管线：解析 manifest → 解析 config_dir（`~` 展开、目录不存在则先创建）
    /// → 实例化校验。只读落位目录，不联网。
    fn load_placed(&self, id: &str) -> Result<LoadedPlugin, String> {
        let placed = install::read(&self.plugin_dir(id)?)?;
        let config_dir = resolve_config_dir(&placed.manifest.config_dir, self.home()?)?;
        instantiate_component(&self.engine, &placed.wasm, &config_dir)?;
        Ok(LoadedPlugin {
            id: placed.manifest.id.clone(),
            name: placed.manifest.name.clone(),
            tool: placed.manifest.tool.clone(),
            config_dir,
            wasm: Arc::from(placed.wasm),
        })
    }

    /// 内置插件的装载管线：内嵌 manifest 校验 → config_dir 解析 → 实例化校验。
    fn load_builtin(&self) -> Result<LoadedPlugin, String> {
        let manifest = parse_manifest(SourceKind::Builtin, builtin::PI_MANIFEST_JSON)
            .map_err(|e| format!("内置插件损坏：{e}"))?;
        let config_dir = resolve_config_dir(&manifest.config_dir, self.home()?)?;
        instantiate_component(&self.engine, builtin::PI_WASM, &config_dir)?;
        Ok(LoadedPlugin {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            tool: manifest.tool.clone(),
            config_dir,
            wasm: Arc::from(builtin::PI_WASM),
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
                    id: entry.id.clone(),
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
        self.rebuild(store);
        Ok(())
    }

    /// 添加 https 插件：先写配置条目（来源重复在 store 层拒绝），随后下载 manifest、
    /// 校验、id 冲突检查、下载 wasm、落位并装载。
    ///
    /// 条目一旦写入即保留：安装失败进错误态并记下原因，用户无需重新填 URL，
    /// 修复后用「重新加载」重试（见 ADR 0006）。
    pub fn add_plugin(&self, store: &mut Store, source: &str) -> Result<(), String> {
        if !is_https_url(source) {
            return Err(format!(
                "插件来源仅支持指向 manifest.json 的 https 地址：{source}"
            ));
        }
        store.add_plugin(source).map_err(|e| e.message())?;
        let outcome = self.install(store, source);
        self.rebuild(store);
        if let Err(reason) = outcome {
            self.record_failure(source, reason);
        }
        Ok(())
    }

    /// 重新加载：「按配置中的来源」无条件重新获取 manifest 与 wasm，成功才替换落位
    /// 目录——失败时旧版本保持可用。每次只作用于一个来源（见 ADR 0006）。
    pub fn reload_plugin(&self, store: &mut Store, source: &str) -> Result<(), String> {
        let entry = store
            .get()
            .map_err(|e| e.message())?
            .plugins
            .iter()
            .find(|plugin| plugin.source == source)
            .ok_or_else(|| {
                StoreError::MissingSource {
                    source: source.to_owned(),
                }
                .message()
            })?;
        if entry.source.starts_with(builtin::SOURCE_PREFIX) {
            return Err("内置插件不可重新加载".to_owned());
        }
        if !is_https_url(&entry.source) {
            return Err(format!("来源 {source} 不是 https 地址，无法重新加载"));
        }
        self.install(store, source)?;
        self.rebuild(store);
        Ok(())
    }

    /// 移除插件：删落位目录与配置条目；两者都已不存在同样成功（幂等）。
    ///
    /// 只删宿主落位的目录，不动用户自己的文件；内置插件不可移除。
    pub fn remove_plugin(&self, store: &mut Store, source: &str) -> Result<(), String> {
        let Some(entry) = store
            .get()
            .map_err(|e| e.message())?
            .plugins
            .iter()
            .find(|plugin| plugin.source == source)
            .cloned()
        else {
            return Ok(());
        };
        if entry.source.starts_with(builtin::SOURCE_PREFIX) {
            return Err("内置插件不可移除".to_owned());
        }
        if let Some(id) = &entry.id {
            install::remove(&self.plugin_dir(id)?)?;
        }
        store.delete_plugin(source).map_err(|e| e.message())?;
        self.rebuild(store);
        Ok(())
    }

    /// 安装与重新加载共用管线：下载 manifest → 校验 → 上游 id 变更检查 → id 冲突检查 →
    /// 下载 wasm → 实例化校验 → 落位 → 持久化 id。
    ///
    /// 实例化校验先于落位：任何失败都不会碰到已落位的旧版本。
    fn install(&self, store: &mut Store, source: &str) -> Result<(), String> {
        let manifest_bytes = self
            .fetcher
            .fetch(source)
            .map_err(|e| format!("下载 manifest 失败：{e}"))?;
        let text = std::str::from_utf8(&manifest_bytes)
            .map_err(|e| format!("manifest.json 不是合法 UTF-8：{e}"))?;
        let manifest = parse_manifest(SourceKind::Https, text)?;
        let installed = self.installed_id(store, source)?;
        check_upstream_id(installed.as_deref(), &manifest.id)?;
        self.check_id_conflict(store, source, &manifest.id)?;

        let wasm = self
            .fetcher
            .fetch(&manifest.entry)
            .map_err(|e| format!("下载插件 wasm 失败：{e}"))?;
        let dir = self.plugin_dir(&manifest.id)?;
        let config_dir = resolve_config_dir(&manifest.config_dir, self.home()?)?;
        instantiate_component(&self.engine, &wasm, &config_dir)?;
        install::place(&dir, text, &wasm)?;

        if installed.as_deref() != Some(manifest.id.as_str())
            && let Err(e) = store.set_plugin_id(source, &manifest.id)
        {
            // 条目 id 是来源到落位目录的唯一映射：映射写不进去，就不能留下
            // 注册表看不见的目录。
            let _ = install::remove(&dir);
            return Err(e.message());
        }
        Ok(())
    }

    /// 条目当前记录的插件 id：`None` 表示该来源尚未安装成功。
    fn installed_id(&self, store: &Store, source: &str) -> Result<Option<String>, String> {
        Ok(store
            .get()
            .map_err(|e| e.message())?
            .plugins
            .iter()
            .find(|plugin| plugin.source == source)
            .and_then(|plugin| plugin.id.clone()))
    }

    /// id 冲突检查：同一 id 只能由一个来源持有。
    fn check_id_conflict(&self, store: &Store, source: &str, id: &str) -> Result<(), String> {
        let config = store.get().map_err(|e| e.message())?;
        match config
            .plugins
            .iter()
            .find(|plugin| plugin.source != source && plugin.id.as_deref() == Some(id))
        {
            Some(other) => Err(format!("插件 id「{id}」已被来源 {} 占用", other.source)),
            None => Ok(()),
        }
    }

    /// 把本次安装失败的原因留在错误态：从磁盘重建注册表时只能按落位状态推导，
    /// 具体原因是网络类还是磁盘类失败只有安装当场知道。
    fn record_failure(&self, source: &str, reason: String) {
        let mut entries = self.entries.lock().unwrap();
        if let Some(entry) = entries.iter_mut().find(|e| e.source == source) {
            entry.state = PluginState::Error(reason);
        }
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
        let mut plugin = instantiate_component(&self.engine, &loaded.wasm, &loaded.config_dir)?;
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

/// 上游 manifest 的 id 变更：报错并保持旧状态，避免静默换成另一个插件。
fn check_upstream_id(installed: Option<&str>, id: &str) -> Result<(), String> {
    match installed {
        Some(old) if old != id => Err(format!(
            "上游 manifest 的插件 id 已从「{old}」变更为「{id}」：已保持旧版本，如需换用请先移除再添加"
        )),
        _ => Ok(()),
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

/// 测试共享助手：临时主目录 + 替身 fetcher 的插件服务与配置存储。
#[cfg(test)]
pub(crate) mod testutil {
    use std::{
        collections::BTreeMap,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };

    use super::{PluginService, fetch::Fetcher, install};
    use crate::store::Store;

    /// 测试替身：按 URL 返回预置响应；未预置的 URL 即失败——
    /// 测试因此绝不会发起真实网络请求。
    #[derive(Default)]
    pub(crate) struct StubFetcher {
        responses: Mutex<BTreeMap<String, Result<Vec<u8>, String>>>,
    }

    impl StubFetcher {
        pub(crate) fn new() -> Self {
            Self::default()
        }

        /// 预置成功响应（覆盖同一 URL 的既有响应，用于模拟上游改版）。
        pub(crate) fn serve(&self, url: &str, body: impl Into<Vec<u8>>) {
            self.responses
                .lock()
                .unwrap()
                .insert(url.to_owned(), Ok(body.into()));
        }

        /// 预置失败响应（网络不可达等）。
        pub(crate) fn fail(&self, url: &str, reason: &str) {
            self.responses
                .lock()
                .unwrap()
                .insert(url.to_owned(), Err(reason.to_owned()));
        }
    }

    impl Fetcher for StubFetcher {
        fn fetch(&self, url: &str) -> Result<Vec<u8>, String> {
            self.responses
                .lock()
                .unwrap()
                .get(url)
                .cloned()
                .unwrap_or_else(|| Err(format!("未预置的 URL：{url}")))
        }
    }

    pub(crate) fn temp_home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    /// 不联网的插件服务：任何未预置的拉取都会失败。
    pub(crate) fn test_service(home: &Path) -> PluginService {
        stub_service(home, StubFetcher::new())
    }

    pub(crate) fn stub_service(home: &Path, fetcher: StubFetcher) -> PluginService {
        PluginService::with_fetcher(Some(home.to_owned()), Arc::new(fetcher))
    }

    pub(crate) fn store_at(home: &Path) -> Store {
        Store::open(home.join(".maestro").join("config.json"))
    }

    /// 测试关心的落位目录：与 install 模块共用同一套布局规则。
    pub(crate) fn placed_dir(home: &Path, id: &str) -> PathBuf {
        install::plugin_dir(&install::root(home), id)
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::{
        StubFetcher, placed_dir, store_at, stub_service, temp_home, test_service,
    };
    use super::*;
    use crate::provider::{Endpoints, ModelEntry};

    const MANIFEST_URL: &str =
        "https://github.com/someone/maestro-plugin-pi2/releases/download/v1/manifest.json";
    const WASM_URL: &str =
        "https://github.com/someone/maestro-plugin-pi2/releases/download/v1/plugin.wasm";

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

    fn manifest_json(id: &str, name: &str, config_dir: &str, entry: &str) -> String {
        format!(
            r#"{{
                "id": "{id}",
                "name": "{name}",
                "tool": "pi",
                "config_dir": "{config_dir}",
                "entry": "{entry}"
            }}"#
        )
    }

    /// 第三方插件的上游 manifest：entry 指向 https 资产，
    /// 产物用真实编译的 pi wasm 充当（内置 pi 已迁移到 SDK，其产物就是符合 WIT 合同的组件）。
    fn third_party_manifest(id: &str, name: &str) -> String {
        manifest_json(id, name, "~/.pi2", WASM_URL)
    }

    fn https_stub(id: &str, name: &str) -> StubFetcher {
        let fetcher = StubFetcher::new();
        fetcher.serve(MANIFEST_URL, third_party_manifest(id, name));
        fetcher.serve(WASM_URL, builtin::PI_WASM.to_vec());
        fetcher
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
    fn unsupported_source_entry_is_reported_as_error_state() {
        let home = temp_home();
        // 旧版本配置可能残留 Git 来源条目：Git 来源已整体废弃（ADR 0006），
        // 进错误态而非静默忽略。
        let path = home.path().join(".maestro").join("config.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"version":1,"providers":{},"plugins":[{"source":"git://example.com/x.git","enabled":true}]}"#,
        )
        .unwrap();
        let store = Mutex::new(Store::open(path));
        let service = test_service(home.path());
        service.startup(&store);

        let views = service.list();
        assert_eq!(views.len(), 2);
        let git = views
            .iter()
            .find(|v| v.source == "git://example.com/x.git")
            .unwrap();
        assert_eq!(git.status, "error");
        assert!(
            git.error.as_deref().unwrap().contains("暂不支持的插件来源"),
            "实际错误：{:?}",
            git.error
        );
    }

    #[test]
    fn https_entry_without_installed_dir_points_at_reload() {
        let home = temp_home();
        let mut store = store_at(home.path());
        store.add_plugin(MANIFEST_URL).unwrap();
        let service = test_service(home.path());

        service.rebuild(&store);

        let view = &service.list()[0];
        assert_eq!(view.status, "error");
        assert_eq!(view.id, None, "从未安装成功：没有落位 id");
        assert!(
            view.error.as_deref().unwrap().contains("「重新加载」重试"),
            "错误态需给出恢复路径：{:?}",
            view.error
        );
    }

    #[test]
    fn id_conflict_marks_later_entry_as_error() {
        let home = temp_home();
        // 手工编辑出的 id 冲突：同一 id 由两个来源持有。
        let path = home.path().join(".maestro").join("config.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                r#"{{"version":1,"providers":{{}},"plugins":[
                    {{"source":"builtin:pi","enabled":true,"id":"pi"}},
                    {{"source":"{MANIFEST_URL}","enabled":true,"id":"pi"}}]}}"#
            ),
        )
        .unwrap();
        let store = Mutex::new(Store::open(path));
        let service = test_service(home.path());
        service.startup(&store);

        let views = service.list();
        assert_eq!(views[0].status, "loaded", "内置插件先装载，占住 id");
        assert_eq!(views[1].status, "error");
        assert_eq!(views[1].id.as_deref(), Some("pi"));
        let error = views[1].error.as_deref().unwrap();
        assert!(error.contains("冲突"), "{error}");
        assert!(error.contains("builtin:pi"), "需指明冲突来源：{error}");
    }

    // ---- 安装与重新加载 ----

    #[test]
    fn add_https_plugin_installs_places_and_projects() {
        let home = temp_home();
        let mut store = store_at(home.path());
        let service = stub_service(home.path(), https_stub("pi2", "Pi2"));

        service.add_plugin(&mut store, MANIFEST_URL).unwrap();

        // 条目：来源即身份，id 在安装成功后落盘（来源 → 落位目录的映射）。
        let plugins = &store.get().unwrap().plugins;
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].source, MANIFEST_URL);
        assert_eq!(plugins[0].id.as_deref(), Some("pi2"));

        // 落位目录：manifest.json + plugin.wasm；manifest 原样保留（entry 仍是上游 URL）。
        let dir = placed_dir(home.path(), "pi2");
        let disk: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(disk["entry"], WASM_URL, "entry 保留回源地址");
        assert_eq!(disk["name"], "Pi2");
        assert!(!fs::read(dir.join("plugin.wasm")).unwrap().is_empty());

        let views = service.list();
        assert_eq!(views[0].status, "loaded", "{:?}", views[0].error);
        assert!(!views[0].builtin, "https 来源可重新加载、可移除");
        assert_eq!(views[0].id.as_deref(), Some("pi2"));
        assert!(views[0].config_dir.as_deref().unwrap().ends_with(".pi2"));

        // 投影：第三方插件与内置插件共用同一装载、校验与沙箱管线。
        store
            .create_provider(
                "gateway",
                provider_openai("https://api.example.com/v1", "sk-plain", vec![]),
            )
            .unwrap();
        let reports = service.apply(&store.get().unwrap().providers);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].status, "applied", "{:?}", reports[0].reason);
        assert_eq!(reports[0].files, vec!["agent/models.json"]);
        let written: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(home.path().join(".pi2").join("agent").join("models.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            written["providers"]["gateway"]["baseUrl"],
            "https://api.example.com/v1"
        );
    }

    #[test]
    fn add_plugin_rejects_non_https_source() {
        let home = temp_home();
        let mut store = store_at(home.path());
        let service = test_service(home.path());

        for source in [
            "http://example.com/manifest.json",
            "file:///tmp/manifest.json",
            "git://example.com/x.git",
            "/tmp/manifest.json",
        ] {
            let err = service.add_plugin(&mut store, source).unwrap_err();
            assert!(err.contains("插件来源仅支持"), "{source} 应被拒绝：{err}");
            assert!(
                !err.contains("未预置的 URL"),
                "仅 https 的判定必须在发起下载之前：{err}"
            );
        }
        assert!(
            store.get().unwrap().plugins.is_empty(),
            "被拒绝的来源不得留下条目"
        );
    }

    #[test]
    fn add_plugin_rejects_duplicate_source_in_store_layer() {
        let home = temp_home();
        let mut store = store_at(home.path());
        let service = stub_service(home.path(), https_stub("pi2", "Pi2"));
        service.add_plugin(&mut store, MANIFEST_URL).unwrap();

        let err = service.add_plugin(&mut store, MANIFEST_URL).unwrap_err();

        assert!(err.contains("已存在同一来源"), "{err}");
        assert_eq!(store.get().unwrap().plugins.len(), 1);
    }

    #[test]
    fn add_plugin_keeps_entry_in_error_state_when_download_fails() {
        let home = temp_home();
        let mut store = store_at(home.path());
        let fetcher = StubFetcher::new();
        fetcher.fail(MANIFEST_URL, "网络不可达");
        let service = stub_service(home.path(), fetcher);

        service.add_plugin(&mut store, MANIFEST_URL).unwrap();

        let plugins = &store.get().unwrap().plugins;
        assert_eq!(plugins.len(), 1, "安装失败保留条目，用户无需重新填 URL");
        assert_eq!(plugins[0].id, None);
        let views = service.list();
        assert_eq!(views[0].status, "error");
        let error = views[0].error.as_deref().unwrap();
        assert!(error.contains("下载 manifest 失败"), "{error}");
        assert!(error.contains("网络不可达"), "原因需可见：{error}");
        assert!(!placed_dir(home.path(), "pi2").exists());
    }

    #[test]
    fn failed_install_can_be_retried_with_reload() {
        let home = temp_home();
        let mut store = store_at(home.path());
        let fetcher = Arc::new(StubFetcher::new());
        fetcher.fail(MANIFEST_URL, "网络不可达");
        let service = PluginService::with_fetcher(Some(home.path().to_owned()), fetcher.clone());
        service.add_plugin(&mut store, MANIFEST_URL).unwrap();
        assert_eq!(service.list()[0].status, "error");

        // 网络恢复后用「重新加载」重试同一来源，不必重新填 URL。
        fetcher.serve(MANIFEST_URL, third_party_manifest("pi2", "Pi2"));
        fetcher.serve(WASM_URL, builtin::PI_WASM.to_vec());
        service.reload_plugin(&mut store, MANIFEST_URL).unwrap();

        assert_eq!(service.list()[0].status, "loaded");
        assert_eq!(store.get().unwrap().plugins[0].id.as_deref(), Some("pi2"));
        assert!(placed_dir(home.path(), "pi2").join("plugin.wasm").exists());
    }

    #[test]
    fn install_rejects_invalid_third_party_manifest_and_config_dir() {
        let home = temp_home();
        let mut store = store_at(home.path());
        let fetcher = StubFetcher::new();
        // entry 不是 https URL：https 来源的 entry 约束与内置插件之外的来源一致从严。
        fetcher.serve(
            MANIFEST_URL,
            manifest_json("pi2", "Pi2", "~/.pi2", "plugin.wasm"),
        );
        let service = stub_service(home.path(), fetcher);
        service.add_plugin(&mut store, MANIFEST_URL).unwrap();
        let view = &service.list()[0];
        assert_eq!(view.status, "error");
        assert!(
            view.error.as_deref().unwrap().contains("https URL"),
            "{:?}",
            view.error
        );

        // config_dir 逃逸主目录：与内置插件同一套安全规则，拒绝且不落位。
        let home = temp_home();
        let mut store = store_at(home.path());
        let fetcher = StubFetcher::new();
        fetcher.serve(
            MANIFEST_URL,
            manifest_json("pi2", "Pi2", "~/.pi2/../../etc", WASM_URL),
        );
        fetcher.serve(WASM_URL, builtin::PI_WASM.to_vec());
        let service = stub_service(home.path(), fetcher);
        service.add_plugin(&mut store, MANIFEST_URL).unwrap();
        let view = &service.list()[0];
        assert_eq!(view.status, "error");
        assert!(
            view.error.as_deref().unwrap().contains("config_dir"),
            "{:?}",
            view.error
        );
        assert!(!placed_dir(home.path(), "pi2").exists());
    }

    #[test]
    fn add_plugin_rejects_id_conflict_with_builtin() {
        let home = temp_home();
        let store = Mutex::new(store_at(home.path()));
        let service = stub_service(home.path(), https_stub("pi", "Pi clone"));
        service.startup(&store);
        let mut store = store.into_inner().unwrap();

        service.add_plugin(&mut store, MANIFEST_URL).unwrap();

        let added = service
            .list()
            .into_iter()
            .find(|view| view.source == MANIFEST_URL)
            .unwrap();
        assert_eq!(added.status, "error");
        let error = added.error.unwrap();
        assert!(error.contains("已被来源 builtin:pi 占用"), "{error}");
        assert_eq!(
            store
                .get()
                .unwrap()
                .plugins
                .iter()
                .find(|plugin| plugin.source == MANIFEST_URL)
                .unwrap()
                .id,
            None,
            "冲突时不得写入 id"
        );
        assert!(
            !placed_dir(home.path(), "pi").exists(),
            "冲突不得落位覆盖既有插件"
        );
    }

    #[test]
    fn installed_plugin_loads_from_disk_offline_after_restart() {
        let home = temp_home();
        let mut store = store_at(home.path());
        let service = stub_service(home.path(), https_stub("pi2", "Pi2"));
        service.add_plugin(&mut store, MANIFEST_URL).unwrap();

        // 模拟重启：全新服务 + 空替身（任何拉取都会失败），只读磁盘重建注册表。
        let offline = test_service(home.path());
        offline.rebuild(&store);

        let views = offline.list();
        assert_eq!(views[0].status, "loaded", "{:?}", views[0].error);
        assert_eq!(views[0].id.as_deref(), Some("pi2"));
        assert_eq!(views[0].name.as_deref(), Some("Pi2"));
    }

    #[test]
    fn reload_keeps_old_version_usable_when_download_or_validation_fails() {
        let home = temp_home();
        let mut store = store_at(home.path());
        let fetcher = Arc::new(StubFetcher::new());
        fetcher.serve(MANIFEST_URL, third_party_manifest("pi2", "Pi2 v1"));
        fetcher.serve(WASM_URL, builtin::PI_WASM.to_vec());
        let service = PluginService::with_fetcher(Some(home.path().to_owned()), fetcher.clone());
        service.add_plugin(&mut store, MANIFEST_URL).unwrap();
        let dir = placed_dir(home.path(), "pi2");
        let installed = fs::read(dir.join("plugin.wasm")).unwrap();

        // 上游 manifest 的 id 变更：报错并保持旧状态。
        fetcher.serve(MANIFEST_URL, third_party_manifest("other", "Other"));
        let err = service.reload_plugin(&mut store, MANIFEST_URL).unwrap_err();
        assert!(err.contains("id 已从"), "{err}");
        assert_eq!(service.list()[0].name.as_deref(), Some("Pi2 v1"));
        assert_eq!(fs::read(dir.join("plugin.wasm")).unwrap(), installed);
        assert!(
            !placed_dir(home.path(), "other").exists(),
            "id 变更不得落位到新 id 目录"
        );

        // wasm 不是有效的组件：装载校验失败，旧目录不被替换。
        fetcher.serve(MANIFEST_URL, third_party_manifest("pi2", "Pi2 v2"));
        fetcher.serve(WASM_URL, b"not a wasm component".to_vec());
        let err = service.reload_plugin(&mut store, MANIFEST_URL).unwrap_err();
        assert!(err.contains("WASM"), "{err}");
        assert_eq!(
            fs::read(dir.join("plugin.wasm")).unwrap(),
            installed,
            "重新加载失败旧版本保持可用"
        );
        assert_eq!(service.list()[0].name.as_deref(), Some("Pi2 v1"));
        assert_eq!(store.get().unwrap().plugins[0].id.as_deref(), Some("pi2"));

        // 重新加载成功：替换落位目录为新版本，不留备份。
        fetcher.serve(WASM_URL, builtin::PI_WASM.to_vec());
        service.reload_plugin(&mut store, MANIFEST_URL).unwrap();
        assert_eq!(service.list()[0].name.as_deref(), Some("Pi2 v2"));
        assert!(
            !dir.with_extension("old").exists(),
            "替换成功后不留备份目录"
        );
    }

    #[test]
    fn reload_rejects_builtin_and_missing_source() {
        let home = temp_home();
        let mut store = store_at(home.path());
        store
            .upsert_builtin_plugin(builtin::BUILTIN_PI_SOURCE, builtin::BUILTIN_PI_ID)
            .unwrap();
        let service = test_service(home.path());

        let err = service
            .reload_plugin(&mut store, builtin::BUILTIN_PI_SOURCE)
            .unwrap_err();
        assert!(err.contains("内置插件不可重新加载"), "{err}");
        let err = service
            .remove_plugin(&mut store, builtin::BUILTIN_PI_SOURCE)
            .unwrap_err();
        assert!(err.contains("内置插件不可移除"), "{err}");
        let err = service
            .reload_plugin(&mut store, "https://nope.example.com/manifest.json")
            .unwrap_err();
        assert!(err.contains("插件条目不存在"), "{err}");
    }

    /// POSIX 权限是唯一能在「条目已写入」之后让配置文件写入失败的手段，
    /// 用来覆盖 id 映射落盘失败时的回滚：不留注册表看不见的落位目录。
    #[cfg(unix)]
    #[test]
    fn failed_id_write_rolls_back_the_placed_dir() {
        use std::os::unix::fs::PermissionsExt;

        let home = temp_home();
        let mut store = store_at(home.path());
        let service = stub_service(home.path(), https_stub("pi2", "Pi2"));
        store.add_plugin(MANIFEST_URL).unwrap();
        let maestro_dir = home.path().join(".maestro");
        fs::create_dir_all(maestro_dir.join("plugins")).unwrap();
        // 配置目录不可写：落位成功，但条目 id 落盘失败。
        fs::set_permissions(&maestro_dir, fs::Permissions::from_mode(0o555)).unwrap();

        let outcome = service.reload_plugin(&mut store, MANIFEST_URL);

        fs::set_permissions(&maestro_dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(outcome.is_err(), "id 落盘失败应报错：{outcome:?}");
        assert!(
            !placed_dir(home.path(), "pi2").exists(),
            "id 未落盘则回滚落位目录，不留注册表看不见的孤儿"
        );
    }

    #[test]
    fn remove_plugin_deletes_entry_and_placed_dir() {
        let home = temp_home();
        let mut store = store_at(home.path());
        let service = stub_service(home.path(), https_stub("pi2", "Pi2"));
        service.add_plugin(&mut store, MANIFEST_URL).unwrap();
        let dir = placed_dir(home.path(), "pi2");
        assert!(dir.exists());

        service.remove_plugin(&mut store, MANIFEST_URL).unwrap();

        assert!(
            store.get().unwrap().plugins.is_empty(),
            "配置条目随移除删除"
        );
        assert!(!dir.exists(), "落位目录随移除删除");
        assert!(service.list().is_empty());
    }

    #[test]
    fn remove_plugin_is_idempotent() {
        let home = temp_home();
        let mut store = store_at(home.path());
        let service = stub_service(home.path(), https_stub("pi2", "Pi2"));
        service.add_plugin(&mut store, MANIFEST_URL).unwrap();
        // 落位目录已被人为删除：移除仍然成功（幂等）。
        fs::remove_dir_all(placed_dir(home.path(), "pi2")).unwrap();

        service.remove_plugin(&mut store, MANIFEST_URL).unwrap();
        // 条目已不存在：视为已移除，不报错。
        service.remove_plugin(&mut store, MANIFEST_URL).unwrap();

        assert!(service.list().is_empty());
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
