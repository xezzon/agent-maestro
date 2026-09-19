//! 插件服务：从配置条目装载插件（内置字节或落位目录）、维护内存注册表、
//! 并把 Provider 投影进各插件声明的配置目录（wasmtime 宿主，WASI 0.2）。
//!
//! 架构决策见 ADR 0004：宿主不代写文件，而是把 manifest 声明的 `config_dir`
//! 预开放给组件（guest 路径 `/`），插件在沙箱内经 WASI 直接落盘。
//! 第三方来源（https 与 file）的获取、落位与生命周期见 ADR 0006。

mod builtin;
mod execution;
mod fetch;
mod manifest;

use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};
use wasmtime::Engine;

use crate::{
    paths::MaestroPaths,
    plugin::builtin::{BUILTIN_PI_ID, BUILTIN_PI_SOURCE},
    provider::Provider,
    store::{AppStore, StoreError},
};
use execution::SkippedProvider;
use fetch::{Fetcher, HttpFetcher};
use manifest::{Manifest, SourceKind, is_https_url};

/// 落位目录中的 manifest 文件名。
pub const PLACED_MANIFEST: &str = "manifest.json";
/// 落位目录中的 wasm 文件名；上游资源内容固定落到该名，装载时只读此名、不解析 `entry`。
pub const PLACED_WASM: &str = "plugin.wasm";

pub struct PlacedPlugin {
    plugin_dir: PathBuf,
    raw_manifest: String,
    wasm: Vec<u8>,
}

pub struct LoadedPlugin {
    manifest: Manifest,
    wasm: Arc<[u8]>,
    plugin: PlacedPlugin,
}

impl PlacedPlugin {
    fn new(plugin_dir: &Path, raw_manifest: String, wasm: &[u8]) -> Self {
        Self {
            plugin_dir: plugin_dir.to_path_buf(),
            raw_manifest,
            wasm: wasm.into(),
        }
    }

    fn from_plugin_dir(plugin_dir: &Path) -> Result<Self, String> {
        let manifest_path = plugin_dir.join(PLACED_MANIFEST);
        let raw_manifest = fs::read_to_string(&manifest_path)
            .map_err(|e| format!("读取 {} 失败：{e}", manifest_path.display()))?;

        let wasm_path = plugin_dir.join(PLACED_WASM);
        let wasm =
            fs::read(&wasm_path).map_err(|e| format!("读取 {} 失败：{e}", wasm_path.display()))?;

        Ok(Self {
            plugin_dir: plugin_dir.to_path_buf(),
            raw_manifest,
            wasm,
        })
    }

    fn save(&self) -> Result<(), String> {
        // 插件根目录
        let plugins_dir = self
            .plugin_dir
            .parent()
            .ok_or_else(|| format!("落位目录不合法：{}", self.plugin_dir.display()))?;
        fs::create_dir_all(plugins_dir).map_err(|e| format!("创建插件目录失败：{e}"))?;
        // 写入临时目录
        let staging = tempfile::Builder::new()
            .prefix(".staging-")
            .tempdir_in(plugins_dir)
            .map_err(|e| format!("创建临时落位目录失败：{e}"))?;
        let staging_path = staging.path().to_owned();
        let io_error = |what: &str| {
            let what = what.to_owned();
            move |e: std::io::Error| format!("写入{what}失败：{e}")
        };
        fs::write(staging_path.join(PLACED_MANIFEST), &self.raw_manifest)
            .map_err(io_error(PLACED_MANIFEST))?;
        fs::write(staging_path.join(PLACED_WASM), self.wasm.clone())
            .map_err(io_error(PLACED_WASM))?;

        match swap_placed(&staging_path, &self.plugin_dir) {
            Ok(()) => {
                // 替换成功后 staging 已改名为目标目录，交由 TempDir 的清理逻辑空跑。
                let _ = staging.keep();
                Ok(())
            }
            // 替换失败：staging 仍留在磁盘上，随 TempDir 一并清理。
            Err(e) => Err(e),
        }
    }

    fn uninstall(&self) -> Result<(), String> {
        match fs::remove_dir_all(&self.plugin_dir) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!(
                "删除插件目录 {} 失败：{e}",
                &self.plugin_dir.display()
            )),
        }
    }

    pub fn load(self) -> Result<LoadedPlugin, String> {
        let manifest = Manifest::try_from(self.raw_manifest.as_str())?;
        let wasm = Arc::from(self.wasm.clone());
        Ok(LoadedPlugin {
            manifest,
            wasm,
            plugin: self,
        })
    }
}

/// 用 staging 目录替换目标目录；目标不存在即直接改名。
///
/// 目标已存在时先备份旧目录，替换成功才删除备份、失败则恢复备份——因此「重新加载」
/// 失败时旧版本保持可用。staging 与目标同父目录，保证替换是一次改名而非跨设备拷贝。
fn swap_placed(staging: &Path, target: &Path) -> Result<(), String> {
    if !target.exists() {
        return fs::rename(staging, target).map_err(|e| format!("落位插件目录失败：{e}"));
    }
    // 插件 id 仅允许 [a-z0-9-_]，故 `<id>.old` 不会与其它插件的落位目录同名。
    let backup = target.with_extension("old");
    if backup.exists() {
        fs::remove_dir_all(&backup).map_err(|e| format!("清理上次落位的备份目录失败：{e}"))?;
    }
    fs::rename(target, &backup).map_err(|e| format!("备份旧版本失败：{e}"))?;
    match fs::rename(staging, target) {
        Ok(()) => {
            // 替换已成功：旧版本删除失败不影响新版本可用。
            let _ = fs::remove_dir_all(&backup);
            Ok(())
        }
        Err(e) => {
            let _ = fs::rename(&backup, target);
            Err(format!("替换插件目录失败：{e}"))
        }
    }
}

/// 插件条目（config.json 的 `plugins` 段，纯增量字段；见 issue #34）。
///
/// `source` 是条目唯一身份（内置 `builtin:<id>`、指向 manifest.json 的 https URL
/// 或本机绝对路径），重复添加在 store 层拒绝；`id` 为插件 id，内置条目在 upsert
/// 时写入，第三方条目在安装成功后写入——它是来源到落位目录的唯一映射（见 ADR 0006）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEntry {
    pub source: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub id: String,
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
    pub id: String,
    pub name: Option<String>,
    pub tool: Option<String>,
    /// manifest 声明的配置目录（`~` 已展开为绝对路径）。
    pub config_dir: Option<String>,
    pub error: Option<String>,
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

/// 注册表条目的装载状态；装载失败进错误态并携带人类可读原因。
enum PluginState {
    /// 损坏的插件
    Error(String),
    /// 禁用的插件，不加载到内存
    Disabled(Manifest),
    /// 已落位且文件没有损坏的插件，可以正常加载
    Loaded(LoadedPlugin),
}

/// 插件服务：内存注册表随配置/磁盘变更整体重建（`rebuild`）。
pub struct PluginService {
    engine: Engine,
    /// 用于展开 manifest 的 `~`；测试可替换为临时主目录。
    maestro_paths: MaestroPaths,
    /// https 拉取的注入点（生产为同步 reqwest 实现，测试为替身）。
    fetcher: Arc<dyn Fetcher>,
    /// 内存注册表：插件 id（配置条目的 `id`）→ 装载状态。
    entries: Mutex<HashMap<String, PluginState>>,
}

/// 引擎配置：启用 fuel 计量（配合 `FUEL_BUDGET` 限制插件执行时长）。
fn build_engine() -> Engine {
    let mut config = wasmtime::Config::new();
    config.consume_fuel(true);
    Engine::new(&config).expect("failed to create wasm engine")
}

impl PluginService {
    pub fn new(maestro_paths: &MaestroPaths) -> Self {
        Self::with_fetcher(maestro_paths, Arc::new(HttpFetcher::new()))
    }

    /// 注入 fetcher：生产用 https 实现，测试用替身（本设计唯一的新缝）。
    pub fn with_fetcher(maestro_paths: &MaestroPaths, fetcher: Arc<dyn Fetcher>) -> Self {
        Self {
            engine: build_engine(),
            maestro_paths: maestro_paths.clone(),
            fetcher,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// 应用启动：从配置中获取插件并逐一加载
    pub fn startup(&self, store: &AppStore) {
        // 从配置文件加载插件
        let store_guard = match store.lock() {
            Ok(guard) => guard,
            Err(_) => {
                eprintln!("failed to lock store during plugin startup");
                return;
            }
        };
        let plugin_entries = match store_guard.list_plugins() {
            Ok(plugin_entries) => plugin_entries,
            Err(_) => {
                eprintln!("");
                return;
            }
        };
        // 将插件加载到内存
        let mut plugins = HashMap::with_capacity(plugin_entries.len());
        for entry in plugin_entries {
            let plugin_id = entry.id.clone();
            let plugin_dir = self.maestro_paths.plugin_dir(&plugin_id);
            let placed_plugin = match PlacedPlugin::from_plugin_dir(&plugin_dir) {
                Ok(placed_plugin) => placed_plugin,
                Err(err) => {
                    plugins.insert(plugin_id, PluginState::Error(err));
                    continue;
                }
            };
            let plugin_state = match placed_plugin.load() {
                Ok(loaded_plugin) => {
                    if entry.enabled {
                        PluginState::Loaded(loaded_plugin)
                    } else {
                        PluginState::Disabled(loaded_plugin.manifest)
                    }
                }
                Err(err) => PluginState::Error(err),
            };
            plugins.insert(plugin_id, plugin_state);
        }

        let builtin_plugins = vec![(BUILTIN_PI_SOURCE, BUILTIN_PI_ID)];

        // 在 plugins 的所有权转移之前，筛选出未写入配置文件的插件
        let nonexistent: Vec<&str> = builtin_plugins
            .iter()
            .filter(|(_, plugin_id)| !plugins.contains_key(plugin_id.to_owned()))
            .map(|(plugin_source, _)| *plugin_source)
            .collect();
        *self.entries.lock().unwrap() = plugins;

        for builtin_plugin in nonexistent {
            let _ = self.add_plugin(store, builtin_plugin);
        }
    }

    /// 当前注册表视图。
    pub fn list(&self, store: &AppStore) -> Result<Vec<PluginView>, String> {
        let plugin_entries = store.lock()?.list_plugins().map_err(|_| "配置文件损坏")?;
        let plugins = self.entries.lock().map_err(|_| "插件未加载")?;
        let plugin_view =
            |entry: &PluginEntry, manifest: Option<&Manifest>, error: Option<String>| {
                let source_kind = SourceKind::from_source(&entry.source);
                PluginView {
                    source: entry.source.clone(),
                    builtin: source_kind == Some(SourceKind::Builtin),
                    enabled: entry.enabled,
                    id: entry.id.clone(),
                    name: manifest.map(|m| m.name.clone()),
                    tool: manifest.map(|m| m.tool.clone()),
                    config_dir: manifest.map(|m| m.config_dir.clone()),
                    error,
                }
            };
        Ok(plugin_entries
            .iter()
            .map(|entry| {
                let plugin = plugins.get(&entry.id);
                match plugin {
                    Some(plugin) => match plugin {
                        PluginState::Loaded(loaded_plugin) => {
                            plugin_view(entry, Some(&loaded_plugin.manifest), None)
                        }
                        PluginState::Disabled(manifest) => {
                            plugin_view(entry, Some(&manifest), None)
                        }
                        PluginState::Error(err) => plugin_view(entry, None, Some(err.to_owned())),
                    },
                    None => plugin_view(entry, None, Some(format!("插件 {} 未加载", entry.id))),
                }
            })
            .collect())
    }

    /// 添加第三方插件：
    /// 1. 先检查来源是否重复。
    /// 2. 随后获取 manifest、校验、id 冲突检查、获取 wasm、落位。
    /// 3. 将插件写入配置条目。（如果之前的步骤失败了，则不写入，而是向前端提示信息）
    ///
    /// 配置存储只在短读/短写处加锁：下载、校验与落位全程不持锁。
    pub fn add_plugin(&self, store: &AppStore, source: &str) -> Result<(), String> {
        // 内置插件随应用分发，不经添加流程（见 ADR 0004）。
        if !matches!(
            SourceKind::from_source(source),
            Some(SourceKind::Https | SourceKind::File)
        ) {
            return Err(format!(
                "插件来源仅支持指向 manifest.json 的 https 地址或本机绝对路径：{source}"
            ));
        }

        // 1. 来源即条目唯一身份：重复添加在发起下载之前拒绝。
        let duplicate = store
            .lock()?
            .list_plugins()?
            .iter()
            .any(|plugin| plugin.source == source);
        if duplicate {
            return Err(StoreError::DuplicateSource {
                source: source.to_owned(),
            }
            .into());
        }

        // 2. 获取 manifest 与 wasm、校验并落位；下载与校验全程不持配置存储锁。
        let mut loaded = self.download(source)?.load()?;
        self.check_id_conflict(store, source, &loaded.manifest.id)?;
        // 装载期即把 config_dir 展开为宿主内的绝对路径：投影按此预开放目录。
        loaded.manifest.config_dir = self
            .resolve_config_dir(&loaded.manifest.config_dir)?
            .display()
            .to_string();
        // 实例化校验先于落位：wasm 不是组件或接口不兼容时不落位、不写条目。
        loaded.instantiate_component(&self.engine)?;
        loaded.plugin.save()?;

        // 3. 安装成功后写入条目：`id` 是来源到落位目录的唯一映射（见 ADR 0006）。
        let entry = PluginEntry {
            source: source.to_owned(),
            enabled: true,
            id: loaded.manifest.id.clone(),
        };
        if let Err(e) = store.lock()?.add_plugin(&entry) {
            // 条目写不进去，就不能留下注册表看不见的落位目录。
            let _ = loaded.plugin.uninstall();
            return Err(e.into());
        }
        self.entries
            .lock()
            .map_err(|_| "插件未加载")?
            .insert(loaded.manifest.id.clone(), PluginState::Loaded(loaded));
        Ok(())
    }

    /// 将插件从来源处拷贝加载到内存
    fn download(&self, source: &str) -> Result<PlacedPlugin, String> {
        let source_kind = SourceKind::from_source(source).ok_or_else(|| "unknown source kind")?;

        let raw_manifest = match source_kind {
            SourceKind::Https => {
                let bytes = self
                    .fetcher
                    .fetch(source)
                    .map_err(|e| format!("下载 manifest 失败：{e}"))?;
                String::from_utf8(bytes)
                    .map_err(|e| format!("manifest.json 不是合法 UTF-8：{e}"))?
            }
            SourceKind::File => {
                fs::read_to_string(source).map_err(|e| format!("读取 {source} 失败：{e}"))?
            }
            SourceKind::Builtin => builtin::PI_MANIFEST_JSON.to_string(),
        };

        let manifest = manifest::parse_manifest(source_kind, &raw_manifest)?;

        let wasm = if source_kind == SourceKind::Builtin {
            Vec::from(builtin::PI_WASM)
        } else if is_https_url(&manifest.entry) {
            self.fetcher.fetch(&manifest.entry)?
        } else {
            // `source` 是 manifest.json 的路径，entry 相对其所在目录解析。
            let source_dir = Path::new(source).parent().unwrap_or(Path::new("."));
            let entry_path = source_dir.join(&manifest.entry);
            fs::read(&entry_path).map_err(|e| format!("读取 {} 失败：{e}", entry_path.display()))?
        };

        Ok(PlacedPlugin::new(
            &self.maestro_paths.plugin_dir(&manifest.id),
            raw_manifest,
            &wasm,
        ))
    }

    /// 启用/禁用插件条目。
    ///
    /// 配置条目是唯一事实来源：先改条目，再同步内存注册表（与 `startup` 的重建
    /// 语义一致）。禁用仅在内存里换成 Disabled 态，保留 manifest 供列表展示；
    /// 启用从落位目录重新装载，装载失败即报错且不改配置，修复落位文件后重新启用即可。
    pub fn set_enabled(&self, store: &AppStore, source: &str, enabled: bool) -> Result<(), String> {
        let entry = store.lock()?.plugin_by_source(source)?;
        if entry.enabled == enabled {
            return Ok(());
        }

        // 启用先装载再写配置：装载失败不写条目、不动注册表，旧状态保持可用。
        let loaded = if enabled {
            let mut loaded =
                PlacedPlugin::from_plugin_dir(&self.maestro_paths.plugin_dir(&entry.id))?.load()?;
            // 装载期把 config_dir 展开为宿主内的绝对路径：投影按此预开放目录。
            loaded.manifest.config_dir = self
                .resolve_config_dir(&loaded.manifest.config_dir)?
                .display()
                .to_string();
            Some(loaded)
        } else {
            None
        };

        store.lock()?.set_plugin_enabled(source, enabled)?;

        let mut entries = self.entries.lock().map_err(|_| "插件未加载")?;
        match loaded {
            Some(loaded) => {
                entries.insert(entry.id.clone(), PluginState::Loaded(loaded));
            }
            None => {
                if let Some(state) = entries.get_mut(&entry.id) {
                    if let PluginState::Loaded(loaded) = state {
                        let manifest = loaded.manifest.clone();
                        *state = PluginState::Disabled(manifest);
                    }
                }
            }
        }
        Ok(())
    }

    /// 移除插件：删落位目录与配置条目；两者都已不存在同样成功（幂等）。
    ///
    /// 只删宿主落位的副本，不动用户的插件项目目录；内置插件不可移除。
    /// 读条目、删落位目录与删条目在同一临界区内：这是配置与磁盘的一次读-改-写，
    /// 中间不得插入另一次安装（重建注册表则放到锁外）。
    pub fn remove_plugin(&self, store: &AppStore, source: &str) -> Result<(), String> {
        todo!()
    }

    pub fn write_providers(
        &self,
        providers: &BTreeMap<String, Provider>,
    ) -> Vec<PluginApplyReport> {
        todo!()
    }

    /// 重新加载：「按配置中的来源」无条件重新获取 manifest 与 wasm，成功才替换落位
    /// 目录——失败时旧版本保持可用。每次只作用于一个来源（见 ADR 0006）。
    pub fn reload_plugin(&self, store: &AppStore, source: &str) -> Result<(), String> {
        todo!()
    }

    /// id 冲突检查：同一 id 只能由一个来源持有。
    fn check_id_conflict(&self, store: &AppStore, source: &str, id: &str) -> Result<(), String> {
        let guard = store.lock()?;
        let config = guard.get()?;
        match config
            .plugins
            .iter()
            .find(|plugin| plugin.source != source && plugin.id.as_str() == id)
        {
            Some(other) => Err(format!("插件 id「{id}」已被来源 {} 占用", other.source)),
            None => Ok(()),
        }
    }

    /// 解析 manifest 声明的配置目录为宿主可控范围内的绝对路径。
    ///
    /// 仅接受 `~/…` 形式（展开为主目录下的相对路径），拒绝绝对路径、`..` 上跳
    /// 与空的 `~`；目录创建后规范化验证仍在主目录内，防止外置 manifest 声明
    /// 任意主机目录并借 WASI 预开放获得读写权限。
    fn resolve_config_dir(&self, config_dir: &str) -> Result<PathBuf, String> {
        let home = &self.maestro_paths.home();
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
}

/// 测试共享助手：临时主目录 + 替身 fetcher 的插件服务与配置存储。
#[cfg(test)]
pub(crate) mod testutil {
    use std::{
        path::Path,
        sync::{Arc, Mutex},
    };

    use super::PluginService;
    pub use super::fetch::testutil::{LockProbeFetcher, StubFetcher};
    pub use super::manifest::testutil::manifest_json;
    use crate::{
        paths::MaestroPaths,
        store::{AppStore, Config, Store},
    };

    pub(crate) fn temp_home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    /// 不联网的插件服务：任何未预置的拉取都会失败。
    pub(crate) fn test_service(home: &Path) -> PluginService {
        stub_service(home, StubFetcher::new())
    }

    pub(crate) fn stub_service(home: &Path, fetcher: StubFetcher) -> PluginService {
        let maestro_paths = MaestroPaths::new(home);
        PluginService::with_fetcher(&maestro_paths, Arc::new(fetcher))
    }

    pub(crate) fn store_at(home: &Path) -> AppStore {
        let maestro_paths = MaestroPaths::new(home);
        AppStore {
            store: Mutex::new(Store::new(&maestro_paths)),
        }
    }

    /// 测试读取配置快照：走与服务同一套短锁访问。
    pub(crate) fn snapshot(store: &AppStore) -> Config {
        store.lock().unwrap().get().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::{
        LockProbeFetcher, StubFetcher, manifest_json, snapshot, store_at, stub_service, temp_home,
        test_service,
    };
    use super::*;
    use crate::paths::MaestroPaths;

    const MANIFEST_URL: &str = "https://example.com/releases/download/v1/manifest.json";
    const WASM_URL: &str = "https://example.com/releases/download/v1/plugin.wasm";

    /// https 来源的替身：manifest 的 entry 指向 Release 资产，产物用内置 pi wasm 充当
    /// （内置 pi 即一份符合 WIT 合同的真实组件）。
    fn https_stub(id: &str, name: &str) -> StubFetcher {
        let fetcher = StubFetcher::new();
        fetcher.serve(
            MANIFEST_URL,
            manifest_json(id, name, "pi", &format!("~/.{id}"), WASM_URL),
        );
        fetcher.serve(WASM_URL, builtin::PI_WASM.to_vec());
        fetcher
    }

    /// 本机插件项目目录的测试替身：写入 manifest.json 与 entry 指向的产物。
    fn source_dir(id: &str, entry: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("manifest.json");
        fs::write(
            &manifest,
            manifest_json(id, "Pi2", "pi", &format!("~/.{id}"), entry),
        )
        .unwrap();
        if !is_https_url(entry) {
            let artifact = dir.path().join(entry);
            fs::create_dir_all(artifact.parent().unwrap()).unwrap();
            fs::write(&artifact, builtin::PI_WASM).unwrap();
        }
        (dir, manifest)
    }

    #[test]
    fn add_https_plugin_writes_entry_places_files_and_loads() {
        let home = temp_home();
        let maestro_paths = MaestroPaths::new(home.path());
        let store = store_at(home.path());
        let service = stub_service(home.path(), https_stub("pi2", "Pi2"));

        service.add_plugin(&store, MANIFEST_URL).unwrap();

        // 条目：id 在安装成功后落盘，是来源到落位目录的唯一映射（见 ADR 0006）。
        let plugins = snapshot(&store).plugins;
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].source, MANIFEST_URL);
        assert_eq!(plugins[0].id, "pi2");
        assert!(plugins[0].enabled);

        // 落位目录：manifest 原样保留上游内容（entry 仍是回源地址）+ 固定名 wasm。
        let dir = maestro_paths.plugin_dir("pi2");
        assert_eq!(
            fs::read_to_string(dir.join(PLACED_MANIFEST)).unwrap(),
            manifest_json("pi2", "Pi2", "pi", "~/.pi2", WASM_URL)
        );
        assert_eq!(
            fs::read(dir.join(PLACED_WASM)).unwrap(),
            builtin::PI_WASM.to_vec()
        );

        let views = service.list(&store).unwrap();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].error, None, "{:?}", views[0].error);
        assert_eq!(views[0].id, "pi2");
        assert!(!views[0].builtin, "https 来源可重新加载、可移除");
        assert_eq!(views[0].name.as_deref(), Some("Pi2"));
        let config_dir = views[0].config_dir.as_deref().unwrap();
        assert!(
            Path::new(config_dir).is_absolute() && config_dir.ends_with(".pi2"),
            "config_dir 已展开为宿主内的绝对路径：{config_dir}"
        );
    }

    #[test]
    fn add_file_plugin_reads_manifest_and_entry_from_the_source_dir() {
        let home = temp_home();
        let maestro_paths = MaestroPaths::new(home.path());
        let store = store_at(home.path());
        let service = test_service(home.path());
        let entry = "target/wasm32-wasip2/release/maestro_plugin_pi.wasm";
        let (source, manifest) = source_dir("pi2", entry);
        let source_id = manifest.display().to_string();

        service.add_plugin(&store, &source_id).unwrap();

        assert_eq!(snapshot(&store).plugins[0].id, "pi2");
        let dir = maestro_paths.plugin_dir("pi2");
        assert_eq!(
            fs::read_to_string(dir.join(PLACED_MANIFEST)).unwrap(),
            fs::read_to_string(&manifest).unwrap(),
            "落位 manifest 为来源原文"
        );
        assert_eq!(
            fs::read(dir.join(PLACED_WASM)).unwrap(),
            builtin::PI_WASM.to_vec()
        );
        assert_eq!(service.list(&store).unwrap()[0].error, None);
        assert!(
            source.path().join(entry).exists(),
            "只删落位副本，不动用户的插件项目目录"
        );
    }

    #[test]
    fn add_plugin_rejects_unrecognized_sources_before_any_io() {
        let home = temp_home();
        let store = store_at(home.path());
        let service = test_service(home.path());

        for source in [
            "http://example.com/manifest.json",
            "file:///tmp/manifest.json",
            "git://example.com/x.git",
            "plugins/pi/manifest.json",
            builtin::BUILTIN_PI_SOURCE,
        ] {
            let err = service.add_plugin(&store, source).unwrap_err();
            assert!(err.contains("插件来源仅支持"), "{source} 应被拒绝：{err}");
            assert!(
                !err.contains("未预置的 URL"),
                "来源形态的判定必须在发起下载之前：{err}"
            );
        }
        assert!(
            snapshot(&store).plugins.is_empty(),
            "被拒绝的来源不得留下条目"
        );
    }

    #[test]
    fn add_plugin_rejects_duplicate_source_before_downloading() {
        let home = temp_home();
        let store = store_at(home.path());
        let fetcher = Arc::new(https_stub("pi2", "Pi2"));
        let service = PluginService::with_fetcher(&MaestroPaths::new(home.path()), fetcher.clone());
        service.add_plugin(&store, MANIFEST_URL).unwrap();
        // 第二次添加：下载若发生必然失败，据此确认重复检查先于下载。
        fetcher.fail(MANIFEST_URL, "网络不可达");

        let err = service.add_plugin(&store, MANIFEST_URL).unwrap_err();

        assert!(err.contains("已存在同一来源"), "{err}");
        assert_eq!(snapshot(&store).plugins.len(), 1);
    }

    #[test]
    fn add_plugin_rejects_id_conflict_without_placing() {
        let home = temp_home();
        let maestro_paths = MaestroPaths::new(home.path());
        fs::create_dir_all(maestro_paths.maestro_dir()).unwrap();
        fs::write(
            maestro_paths.config_path(),
            format!(
                r#"{{"version":1,"providers":{{}},"plugins":[{{"source":"{}","enabled":true,"id":"pi"}}]}}"#,
                builtin::BUILTIN_PI_SOURCE
            ),
        )
        .unwrap();
        let store = store_at(home.path());
        let service = stub_service(home.path(), https_stub("pi", "Pi clone"));

        let err = service.add_plugin(&store, MANIFEST_URL).unwrap_err();

        assert!(err.contains("已被来源 builtin:pi 占用"), "{err}");
        assert_eq!(snapshot(&store).plugins.len(), 1, "冲突不得写条目");
        assert!(!maestro_paths.plugin_dir("pi").exists(), "冲突不得落位");
    }

    #[test]
    fn add_plugin_writes_nothing_when_download_fails() {
        let home = temp_home();
        let maestro_paths = MaestroPaths::new(home.path());
        let store = store_at(home.path());
        let fetcher = StubFetcher::new();
        fetcher.fail(MANIFEST_URL, "网络不可达");
        let service = stub_service(home.path(), fetcher);

        let err = service.add_plugin(&store, MANIFEST_URL).unwrap_err();

        assert!(
            err.contains("下载 manifest 失败") && err.contains("网络不可达"),
            "{err}"
        );
        assert!(snapshot(&store).plugins.is_empty(), "安装失败不得写条目");
        assert!(!maestro_paths.plugins_dir().exists(), "安装失败不得落位");
    }

    #[test]
    fn add_plugin_rejects_wasm_that_is_not_a_component() {
        let home = temp_home();
        let maestro_paths = MaestroPaths::new(home.path());
        let store = store_at(home.path());
        let fetcher = StubFetcher::new();
        fetcher.serve(
            MANIFEST_URL,
            manifest_json("pi2", "Pi2", "pi", "~/.pi2", WASM_URL),
        );
        fetcher.serve(WASM_URL, b"not a wasm component".to_vec());
        let service = stub_service(home.path(), fetcher);

        let err = service.add_plugin(&store, MANIFEST_URL).unwrap_err();

        assert!(err.contains("不是有效的 WASM 组件"), "{err}");
        assert!(snapshot(&store).plugins.is_empty());
        assert!(
            !maestro_paths.plugin_dir("pi2").exists(),
            "校验失败不得落位"
        );
    }

    #[test]
    fn add_plugin_rejects_config_dir_escaping_home() {
        let home = temp_home();
        let maestro_paths = MaestroPaths::new(home.path());
        let store = store_at(home.path());
        let fetcher = StubFetcher::new();
        fetcher.serve(
            MANIFEST_URL,
            manifest_json("pi2", "Pi2", "pi", "~/.pi2/../../etc", WASM_URL),
        );
        fetcher.serve(WASM_URL, builtin::PI_WASM.to_vec());
        let service = stub_service(home.path(), fetcher);

        let err = service.add_plugin(&store, MANIFEST_URL).unwrap_err();

        assert!(err.contains("config_dir"), "{err}");
        assert!(snapshot(&store).plugins.is_empty());
        assert!(!maestro_paths.plugin_dir("pi2").exists());
    }

    /// 下载可能持续数秒：全程不得持有配置存储锁，否则会连锁阻塞所有 Provider / 插件命令。
    #[test]
    fn add_plugin_does_not_hold_the_store_lock_while_downloading() {
        let home = temp_home();
        let store = Arc::new(store_at(home.path()));
        let service = PluginService::with_fetcher(
            &MaestroPaths::new(home.path()),
            Arc::new(LockProbeFetcher::new(
                Arc::clone(&store),
                https_stub("pi2", "Pi2"),
            )),
        );

        service.add_plugin(&store, MANIFEST_URL).unwrap();

        assert!(service.list(&store).unwrap()[0].error.is_none());
    }

    #[test]
    fn set_enabled_disables_and_re_enables_the_plugin() {
        let home = temp_home();
        let store = store_at(home.path());
        let service = stub_service(home.path(), https_stub("pi2", "Pi2"));
        service.add_plugin(&store, MANIFEST_URL).unwrap();

        service.set_enabled(&store, MANIFEST_URL, false).unwrap();
        assert!(!snapshot(&store).plugins[0].enabled);
        let view = &service.list(&store).unwrap()[0];
        assert!(!view.enabled);
        assert_eq!(view.error, None);
        assert_eq!(
            view.name.as_deref(),
            Some("Pi2"),
            "禁用保留 manifest 供展示"
        );

        service.set_enabled(&store, MANIFEST_URL, true).unwrap();
        assert!(snapshot(&store).plugins[0].enabled);
        let view = &service.list(&store).unwrap()[0];
        assert!(view.enabled);
        assert_eq!(view.error, None);
        assert_eq!(view.name.as_deref(), Some("Pi2"));
    }

    #[test]
    fn set_enabled_enable_fails_without_touching_config_when_placed_files_are_missing() {
        let home = temp_home();
        let maestro_paths = MaestroPaths::new(home.path());
        let store = store_at(home.path());
        let service = stub_service(home.path(), https_stub("pi2", "Pi2"));
        service.add_plugin(&store, MANIFEST_URL).unwrap();
        service.set_enabled(&store, MANIFEST_URL, false).unwrap();
        fs::remove_dir_all(maestro_paths.plugin_dir("pi2")).unwrap();

        let err = service.set_enabled(&store, MANIFEST_URL, true).unwrap_err();

        assert!(err.contains("读取"), "{err}");
        assert!(
            !snapshot(&store).plugins[0].enabled,
            "装载失败不得改配置条目"
        );
        assert!(!service.list(&store).unwrap()[0].enabled);
    }

    #[test]
    fn set_enabled_unknown_source_is_rejected() {
        let home = temp_home();
        let store = store_at(home.path());
        let service = test_service(home.path());

        let err = service
            .set_enabled(&store, "builtin:ghost", true)
            .unwrap_err();

        assert!(err.contains("插件条目不存在"), "{err}");
    }
}
