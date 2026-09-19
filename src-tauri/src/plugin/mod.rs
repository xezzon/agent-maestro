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
    provider::Provider,
    store::{AppStore, StoreError},
};
use execution::SkippedProvider;
use fetch::{Fetcher, HttpFetcher};
use manifest::{Manifest, SourceKind, is_https_url, parse_manifest};

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
        *self.entries.lock().unwrap() = plugins;
        // 检查内置插件是否缺失，如有缺失，则添加
        todo!()
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

    /// 启用/禁用插件条目。
    pub fn set_enabled(&self, store: &AppStore, source: &str, enabled: bool) -> Result<(), String> {
        todo!()
    }

    /// 添加第三方插件：
    /// 1. 先检查来源是否重复。
    /// 2. 随后获取 manifest、校验、id 冲突检查、获取 wasm、落位。
    /// 3. 将插件写入配置条目。（如果之前的步骤失败了，则不写入，而是向前端提示信息）
    ///
    /// 配置存储只在短读/短写处加锁：下载、校验与落位全程不持锁。
    pub fn add_plugin(&self, store: &AppStore, source: &str) -> Result<(), String> {
        todo!()
    }

    /// 重新加载：「按配置中的来源」无条件重新获取 manifest 与 wasm，成功才替换落位
    /// 目录——失败时旧版本保持可用。每次只作用于一个来源（见 ADR 0006）。
    pub fn reload_plugin(&self, store: &AppStore, source: &str) -> Result<(), String> {
        todo!()
    }

    /// 将插件从来源处拷贝加载到内存
    fn download(&self, source: &str) -> Result<PlacedPlugin, String> {
        let source_kind = SourceKind::from_source(source).ok_or_else(|| "unknown source kink")?;

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
            let entry_path = PathBuf::from(source).join(&manifest.entry);
            fs::read(&entry_path).map_err(|e| format!("读取 {} 失败: {e}", entry_path.display()))?
        };

        Ok(PlacedPlugin {
            plugin_dir: self.maestro_paths.plugin_dir(&manifest.id),
            raw_manifest,
            wasm,
        })
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
