//! 插件服务：从配置条目装载插件（内置字节 / 磁盘目录）、维护内存注册表、
//! 并把 Provider 投影进各插件声明的配置目录（wasmtime 宿主，WASI 0.2）。
//!
//! 架构决策见 ADR 0004：宿主不代写文件，而是把 manifest 声明的 `config_dir`
//! 预开放给组件（guest 路径 `/`），插件在沙箱内经 WASI 直接落盘。

pub mod builtin;
pub mod git;
pub mod manifest;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::Serialize;
use wasmtime::{
    Engine,
    component::{Component, Linker},
};
use wasmtime_wasi::{FsPerms, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView, p2};

use crate::{
    provider::Provider,
    store::{PluginEntry, Store},
};

mod bindings {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "plugin-world",
    });
}

use manifest::{Manifest, parse_manifest};

use bindings::exports::maestro::plugin::plugin::Protocol as WitProtocol;
/// WIT 合同 v1 的类型化绑定（宿主侧）。
use bindings::exports::maestro::plugin::plugin::{Model as WitModel, Provider as WitProvider};

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

/// 插件 wasm 字节来源：内置插件不物化到磁盘，Git 插件读安装目录的 entry 文件。
#[derive(Debug, Clone)]
enum WasmSource {
    Builtin(&'static [u8]),
    Disk(PathBuf),
}

/// 成功装载的插件（metadata + wasm 来源 + 已展开的配置目录）。
#[derive(Debug, Clone)]
struct LoadedPlugin {
    id: String,
    name: String,
    tool: String,
    config_dir: PathBuf,
    wasm: WasmSource,
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
    /// 用于展开 manifest 的 `~` 与定位 `~/.maestro/plugins`；测试可替换为临时主目录。
    home: Option<PathBuf>,
    entries: Mutex<Vec<RegistryEntry>>,
    /// 安装/更新失败的最近原因（内存态）：条目尚未解析出 id 时用于错误态展示。
    last_errors: Mutex<HashMap<String, String>>,
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

/// 实例化组件（编译通过 + 导出接口匹配 + config_dir 可开放）；
/// 装载时用于兼容性校验，投影时用于实际调用。
fn instantiate_component(
    engine: &Engine,
    bytes: &[u8],
    config_dir: &Path,
) -> Result<InstantiatedPlugin, String> {
    let component =
        Component::new(engine, bytes).map_err(|e| format!("不是有效的 WASM 组件：{e}"))?;
    let mut store = wasmtime::Store::new(engine, HostState::new(config_dir)?);
    let mut linker = Linker::new(engine);
    p2::add_to_linker_sync(&mut linker).map_err(|e| format!("初始化 WASI 宿主环境失败：{e}"))?;
    let world = bindings::PluginWorld::instantiate(&mut store, &component, &linker)
        .map_err(|e| format!("插件接口不兼容：{e}"))?;
    Ok(InstantiatedPlugin { store, world })
}

impl PluginService {
    pub fn new(home: Option<PathBuf>) -> Self {
        Self {
            engine: Engine::default(),
            home,
            entries: Mutex::new(Vec::new()),
            last_errors: Mutex::new(HashMap::new()),
        }
    }

    fn home(&self) -> Result<&Path, String> {
        self.home
            .as_deref()
            .ok_or_else(|| "无法确定用户主目录（HOME）".to_owned())
    }

    /// 插件安装根目录：`~/.maestro/plugins`。
    fn plugins_root(&self) -> Result<PathBuf, String> {
        Ok(self.home()?.join(".maestro").join("plugins"))
    }

    /// 应用启动：upsert 内置条目（离线、只读磁盘）并重建注册表。
    pub fn startup(&self, store: &mut Store) {
        if let Err(e) =
            store.upsert_builtin_plugin(builtin::BUILTIN_PI_SOURCE, builtin::BUILTIN_PI_ID)
        {
            eprintln!("failed to upsert builtin plugin entry: {}", e.message());
        }
        self.reload(store);
    }

    /// 只读磁盘重建注册表，不联网（手动修复插件文件后无需重启应用）。
    pub fn reload(&self, store: &Store) {
        let entries = store
            .get()
            .map(|config| config.plugins.clone())
            .unwrap_or_default();
        let mut seen_ids: HashSet<String> = HashSet::new();
        let built = entries
            .into_iter()
            .map(|entry| self.build_entry(entry, &mut seen_ids))
            .collect();
        *self.entries.lock().unwrap() = built;
    }

    /// 按配置条目顺序装载；不同来源解析出相同插件 id 时后者进错误态。
    fn build_entry(&self, entry: PluginEntry, seen_ids: &mut HashSet<String>) -> RegistryEntry {
        let builtin = entry.source.starts_with("builtin:");
        let state = match self.load_entry(&entry, builtin) {
            Ok(loaded) => {
                if !seen_ids.insert(loaded.id.clone()) {
                    PluginState::Error(format!("插件 id「{}」与其他来源冲突", loaded.id))
                } else {
                    PluginState::Loaded(loaded)
                }
            }
            Err(reason) => PluginState::Error(reason),
        };
        RegistryEntry {
            source: entry.source,
            builtin,
            enabled: entry.enabled,
            state,
        }
    }

    fn load_entry(&self, entry: &PluginEntry, builtin: bool) -> Result<LoadedPlugin, String> {
        if builtin {
            return self.load_builtin();
        }
        match &entry.id {
            Some(id) => self.load_installed(id),
            None => {
                // 先落配置条目、下载失败条目保留：错误态展示最近失败原因，可用「更新」重试。
                let reason = self
                    .last_errors
                    .lock()
                    .unwrap()
                    .get(&entry.source)
                    .cloned()
                    .unwrap_or_else(|| "插件尚未成功安装，请尝试「更新」".to_owned());
                Err(reason)
            }
        }
    }

    fn load_builtin(&self) -> Result<LoadedPlugin, String> {
        let manifest =
            parse_manifest(builtin::PI_MANIFEST_JSON).map_err(|e| format!("内置插件损坏：{e}"))?;
        let loaded =
            self.load_from_manifest(&manifest, WasmSource::Builtin(builtin::PI_WASM), None)?;
        Ok(loaded)
    }

    /// 从安装目录 `~/.maestro/plugins/<id>` 装载。
    fn load_installed(&self, id: &str) -> Result<LoadedPlugin, String> {
        let root = self.plugins_root()?.join(id);
        let text = fs::read_to_string(root.join("manifest.json"))
            .map_err(|_| format!("插件目录损坏：{} 缺少 manifest.json", root.display()))?;
        let manifest = parse_manifest(&text)?;
        if manifest.id != id {
            return Err(format!(
                "插件目录损坏：目录名 {id} 与 manifest 声明的 id「{}」不一致",
                manifest.id
            ));
        }
        self.load_from_manifest(&manifest, WasmSource::Disk(root.clone()), Some(&root))
    }

    /// 装载管线：解析 config_dir（`~` 展开、目录不存在则先创建）→ 读入口字节 → 实例化校验。
    fn load_from_manifest(
        &self,
        manifest: &Manifest,
        wasm: WasmSource,
        plugin_root: Option<&Path>,
    ) -> Result<LoadedPlugin, String> {
        let config_dir = expand_home(&manifest.config_dir, self.home()?)?;
        fs::create_dir_all(&config_dir).map_err(|e| format!("创建插件配置目录失败：{e}"))?;
        let bytes: Vec<u8> = match (&wasm, plugin_root) {
            (WasmSource::Builtin(bytes), _) => bytes.to_vec(),
            (WasmSource::Disk(_), Some(root)) => {
                let entry = root.join(&manifest.entry);
                fs::read(&entry).map_err(|_| format!("插件入口文件缺失：{}", entry.display()))?
            }
            (WasmSource::Disk(_), None) => unreachable!("磁盘插件必须携带插件根目录"),
        };
        instantiate_component(&self.engine, &bytes, &config_dir)?;
        Ok(LoadedPlugin {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            tool: manifest.tool.clone(),
            config_dir,
            wasm,
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

    /// 添加 Git 来源插件：先落配置条目（store 层拒绝重复来源），
    /// 随后克隆/校验/落位/加载；失败保留条目、插件进错误态。
    pub fn add(&self, store: &mut Store, source: &str) -> Result<(), String> {
        store.add_plugin(source).map_err(|e| e.message())?;
        match self.install(store, source, None) {
            Ok(()) => {
                self.last_errors.lock().unwrap().remove(source);
                Ok(())
            }
            Err(reason) => {
                self.last_errors
                    .lock()
                    .unwrap()
                    .insert(source.to_owned(), reason.clone());
                self.reload(store);
                Err(reason)
            }
        }
    }

    /// 更新 Git 来源插件：重新克隆最新默认分支，校验通过才替换旧目录；
    /// 上游 manifest 的 id 变更则报错并保持旧状态。
    pub fn update(&self, store: &mut Store, source: &str) -> Result<(), String> {
        if source.starts_with("builtin:") {
            return Err("内置插件随应用分发，无需更新".to_owned());
        }
        let entry_id = store
            .get()
            .map_err(|e| e.message())?
            .plugins
            .iter()
            .find(|p| p.source == source)
            .and_then(|p| p.id.clone())
            .ok_or_else(|| format!("插件条目不存在：{source}"))?;
        self.install(store, source, Some(&entry_id))?;
        self.last_errors.lock().unwrap().remove(source);
        Ok(())
    }

    /// 克隆 → 读 manifest 校验 → id 冲突检查 → 落位 → 加载。
    /// `expected_id` 为 `Some` 时即更新语义：上游 id 变更则报错并保持旧状态。
    fn install(
        &self,
        store: &mut Store,
        source: &str,
        expected_id: Option<&str>,
    ) -> Result<(), String> {
        let plugins_root = self.plugins_root()?;
        let staging = git::clone_to_staging(source, &plugins_root)?;
        let manifest = git::read_staging_manifest(staging.path())?;
        if let Some(expected) = expected_id
            && manifest.id != expected
        {
            return Err(format!(
                "上游插件 id 已变更为「{}」，与现有插件「{expected}」不一致，已保持旧版本",
                manifest.id
            ));
        }
        self.check_id_conflict(store, source, &manifest.id)?;
        self.validate_staging_wasm(staging.path(), &manifest)?;
        git::install_staging(staging, &plugins_root, &manifest.id)?;
        store
            .set_plugin_id(source, &manifest.id)
            .map_err(|e| e.message())?;
        self.reload(store);
        Ok(())
    }

    /// 落位前先实例化校验暂存目录的入口 wasm：
    /// 更新失败（含新版接口不兼容）时旧目录保持原样，旧版本继续可用。
    fn validate_staging_wasm(&self, staging: &Path, manifest: &Manifest) -> Result<(), String> {
        let config_dir = expand_home(&manifest.config_dir, self.home()?)?;
        fs::create_dir_all(&config_dir).map_err(|e| format!("创建插件配置目录失败：{e}"))?;
        let entry = staging.join(&manifest.entry);
        let bytes =
            fs::read(&entry).map_err(|_| format!("插件入口文件缺失：{}", entry.display()))?;
        instantiate_component(&self.engine, &bytes, &config_dir)?;
        Ok(())
    }

    /// 插件 id 是全局身份：与其他来源（含内置）的已解析 id 冲突即拒绝落位。
    fn check_id_conflict(&self, store: &Store, source: &str, id: &str) -> Result<(), String> {
        let conflict = store
            .get()
            .map_err(|e| e.message())?
            .plugins
            .iter()
            .any(|p| p.source != source && p.id.as_deref() == Some(id));
        if conflict {
            return Err(format!("插件 id「{id}」已被其他来源占用"));
        }
        Ok(())
    }

    /// 移除 Git 插件：删配置条目 + 删插件目录（幂等）。内置插件不可移除。
    pub fn remove(&self, store: &mut Store, source: &str) -> Result<(), String> {
        if source.starts_with("builtin:") {
            return Err("内置插件可禁用但不可移除".to_owned());
        }
        let id = store
            .get()
            .map_err(|e| e.message())?
            .plugins
            .iter()
            .find(|p| p.source == source)
            .and_then(|p| p.id.clone());
        store.remove_plugin(source).map_err(|e| e.message())?;
        if let Some(id) = id {
            // 幂等：目录已不存在视为成功。
            let _ = fs::remove_dir_all(self.plugins_root()?.join(id));
        }
        self.last_errors.lock().unwrap().remove(source);
        self.reload(store);
        Ok(())
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
        let bytes = match &loaded.wasm {
            WasmSource::Builtin(bytes) => bytes.to_vec(),
            WasmSource::Disk(entry) => {
                fs::read(entry).map_err(|e| format!("读取插件入口文件失败：{e}"))?
            }
        };
        let mut plugin = instantiate_component(&self.engine, &bytes, &loaded.config_dir)?;
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

/// 展开 manifest config_dir 的 `~` 前缀为主目录。
fn expand_home(config_dir: &str, home: &Path) -> Result<PathBuf, String> {
    if config_dir == "~" {
        return Ok(home.to_owned());
    }
    if let Some(rest) = config_dir.strip_prefix("~/") {
        return Ok(home.join(rest));
    }
    Ok(PathBuf::from(config_dir))
}

/// 测试共享助手：临时主目录 + 独立引擎的插件服务与配置存储。
#[cfg(test)]
pub(crate) mod testutil {
    use crate::store::Store;
    use std::path::Path;

    use super::PluginService;

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
        let mut store = store_at(home);
        let service = test_service(home);
        service.startup(&mut store);
        service
    }

    fn write_installed_plugin(home: &Path, id: &str, manifest: &str, wasm: &[u8]) {
        let root = home.join(".maestro").join("plugins").join(id);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("manifest.json"), manifest).unwrap();
        fs::write(root.join("plugin.wasm"), wasm).unwrap();
    }

    const GOOD_MANIFEST: &str = r#"{
        "id": "sample",
        "name": "Sample",
        "tool": "sample",
        "config_dir": "~/.sample",
        "entry": "plugin.wasm"
    }"#;

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
        let mut store = store_at(home.path());
        let service = test_service(home.path());
        service.startup(&mut store);

        // 启动 upsert 落盘；重启（重新 open）后条目仍在。
        let reopened = Store::open(home.path().join(".maestro").join("config.json"));
        let plugins = &reopened.get().unwrap().plugins;
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].source, builtin::BUILTIN_PI_SOURCE);
        assert_eq!(plugins[0].id.as_deref(), Some("pi"));
    }

    #[test]
    fn missing_entry_file_is_reported_as_error_state() {
        let home = temp_home();
        let root = home.path().join(".maestro").join("plugins").join("sample");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("manifest.json"), GOOD_MANIFEST).unwrap();

        let mut store = store_at(home.path());
        store.add_plugin("https://example.com/sample.git").unwrap();
        store
            .set_plugin_id("https://example.com/sample.git", "sample")
            .unwrap();

        let service = test_service(home.path());
        service.reload(&store);

        let views = service.list();
        assert_eq!(views[0].status, "error");
        assert!(
            views[0]
                .error
                .as_deref()
                .unwrap()
                .contains("插件入口文件缺失")
        );
    }

    #[test]
    fn invalid_manifest_is_reported_as_error_state() {
        let home = temp_home();
        let root = home.path().join(".maestro").join("plugins").join("sample");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("manifest.json"), r#"{"id": "sample"}"#).unwrap();
        fs::write(root.join("plugin.wasm"), b"whatever").unwrap();

        let mut store = store_at(home.path());
        store.add_plugin("https://example.com/sample.git").unwrap();
        store
            .set_plugin_id("https://example.com/sample.git", "sample")
            .unwrap();

        let service = test_service(home.path());
        service.reload(&store);

        let views = service.list();
        assert_eq!(views[0].status, "error");
        assert!(
            views[0]
                .error
                .as_deref()
                .unwrap()
                .contains("manifest.json 不合法")
        );
    }

    #[test]
    fn incompatible_wasm_is_reported_as_error_state() {
        let home = temp_home();
        // 合法组件但导出接口与合同不符（实例化时才发现，而非编译失败）。
        // Component::new 支持 wat 文本，测试插件直接以 wat 文本落盘。
        write_installed_plugin(
            home.path(),
            "sample",
            GOOD_MANIFEST,
            br#"(component
                (import "maestro:plugin/plugin@1.0.0" (func))
            )"#,
        );

        let mut store = store_at(home.path());
        store.add_plugin("https://example.com/sample.git").unwrap();
        store
            .set_plugin_id("https://example.com/sample.git", "sample")
            .unwrap();

        let service = test_service(home.path());
        service.reload(&store);

        let views = service.list();
        assert_eq!(views[0].status, "error");
        assert!(
            views[0]
                .error
                .as_deref()
                .unwrap()
                .contains("插件接口不兼容"),
            "实际错误：{:?}",
            views[0].error
        );
    }

    #[test]
    fn not_a_component_is_reported_as_error_state() {
        let home = temp_home();
        // 普通 core module 不是组件，编译阶段即失败。
        write_installed_plugin(home.path(), "sample", GOOD_MANIFEST, br#"(module)"#);

        let mut store = store_at(home.path());
        store.add_plugin("https://example.com/sample.git").unwrap();
        store
            .set_plugin_id("https://example.com/sample.git", "sample")
            .unwrap();

        let service = test_service(home.path());
        service.reload(&store);

        let views = service.list();
        assert_eq!(views[0].status, "error");
        assert!(
            views[0]
                .error
                .as_deref()
                .unwrap()
                .contains("不是有效的 WASM 组件")
        );
    }

    #[test]
    fn id_conflict_puts_later_source_into_error_state() {
        let home = temp_home();
        // 已安装目录声明与内置插件相同的 id「pi」。
        let pi_clone_manifest = r#"{
            "id": "pi",
            "name": "Pi Clone",
            "tool": "pi",
            "config_dir": "~/.pi-clone",
            "entry": "plugin.wasm"
        }"#;
        write_installed_plugin(home.path(), "pi", pi_clone_manifest, builtin::PI_WASM);
        let mut store = store_at(home.path());
        store
            .add_plugin("https://example.com/pi-clone.git")
            .unwrap();
        store
            .set_plugin_id("https://example.com/pi-clone.git", "pi")
            .unwrap();

        let service = test_service(home.path());
        // startup 会先 upsert 内置条目（先加载），克隆来源后加载 → 后者进错误态。
        service.startup(&mut store);

        let views = service.list();
        assert_eq!(views.len(), 2);
        assert_eq!(views[0].status, "loaded", "先加载者保持正常");
        assert_eq!(views[1].status, "error");
        assert!(views[1].error.as_deref().unwrap().contains("冲突"));
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
        let mut store = store_at(home.path());
        let service = test_service(home.path());
        service.startup(&mut store);
        service
            .set_enabled(&mut store, builtin::BUILTIN_PI_SOURCE, false)
            .unwrap();

        let reports = service.apply(&BTreeMap::new());
        assert_eq!(reports[0].status, "skipped");
        assert_eq!(reports[0].reason.as_deref(), Some("已禁用"));
    }

    #[test]
    fn errored_plugin_does_not_block_other_plugins() {
        let home = temp_home();
        // 损坏插件（非组件）+ 可用内置 pi：前者失败不影响后者。
        write_installed_plugin(home.path(), "broken", GOOD_MANIFEST, br#"(module)"#);
        let mut store = store_at(home.path());
        store.add_plugin("https://example.com/broken.git").unwrap();
        store
            .set_plugin_id("https://example.com/broken.git", "broken")
            .unwrap();

        let service = test_service(home.path());
        service.startup(&mut store);

        let mut providers = BTreeMap::new();
        providers.insert(
            "gateway".to_owned(),
            provider_openai("https://api.example.com/v1", "", vec![]),
        );

        let reports = service.apply(&providers);
        assert_eq!(
            reports.len(),
            2,
            "注册表按配置条目顺序：broken 先于内置（upsert 后插入）"
        );

        let broken = reports
            .iter()
            .find(|r| r.id.is_none() || r.status != "applied")
            .unwrap();
        assert_eq!(broken.status, "skipped");
        assert!(broken.reason.as_deref().unwrap().contains("插件加载失败"));

        let pi = reports.iter().find(|r| r.status == "applied").unwrap();
        assert_eq!(pi.files, vec!["agent/models.json"]);
        assert_eq!(pi.id.as_deref(), Some("pi"));
    }
}
