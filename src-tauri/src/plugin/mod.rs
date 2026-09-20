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
    sync::{Arc, Mutex, RwLock},
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
/// 落位目录中的 wasm 文件名；上游资源内容固定落到该名。装载时 `entry` 仍会被
/// 反序列化，但其指向的路径不再被解析——落位产物只认这个固定文件名。
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

    /// 落位目录的绝对路径（暴露给错误信息，便于用户手动清理）。
    pub fn path(&self) -> &Path {
        &self.plugin_dir
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
        remove_placed_dir(&self.plugin_dir)
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

/// 删除落位目录；目录已不存在同样成功（幂等）。
fn remove_placed_dir(plugin_dir: &Path) -> Result<(), String> {
    match fs::remove_dir_all(plugin_dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("删除插件目录 {} 失败：{e}", plugin_dir.display())),
    }
}

/// 插件条目（config.json 的 `plugins` 段，纯增量字段；见 issue #34）。
///
/// `source` 是条目唯一身份（内置 `builtin:<id>`、指向 manifest.json 的 https URL
/// 或本机绝对路径），重复添加在 store 层拒绝；`id` 为插件 id，条目在安装成功后
/// 写入（内置插件由启动时装载管线补写，与其余来源同一流程）——它是来源到落位
/// 目录的唯一映射（见 ADR 0006）。
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
    /// 来源以内置前缀标识；内置插件可禁用、不可移除。
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
    pub id: String,
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
    /// 已落位且文件没有损坏的插件，可以正常加载。
    /// Arc 便于投影前克隆快照、锁外执行 wasm（见 `write_providers`）。
    Loaded(Arc<LoadedPlugin>),
}

/// 插件服务：内存注册表由 `startup` 与各生命周期方法（安装/启停/移除/重载）
/// 按配置条目增量维护。
pub struct PluginService {
    engine: Engine,
    /// 用于展开 manifest 的 `~`；测试可替换为临时主目录。
    maestro_paths: MaestroPaths,
    /// https 拉取的注入点（生产为同步 reqwest 实现，测试为替身）。
    fetcher: Arc<dyn Fetcher>,
    /// 内存注册表：插件 id（配置条目的 `id`）→ 装载状态。
    /// 读多写少：`list` / `write_providers` 的快照读路径在锁内并发；
    /// 安装/启停/移除/重载写路径走 `entries.write()` 独占。
    entries: RwLock<HashMap<String, PluginState>>,
    /// install / reload / remove 共用的独占临界区：把"检查 → 落位 → 写条目 →
    /// 同步注册表 → 删落位目录"作为一个不可分割的步骤，避免同 id 的并发
    /// install 各自落位后又被对方的清理逻辑误删。
    install_lock: Mutex<()>,
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
            entries: RwLock::new(HashMap::new()),
            install_lock: Mutex::new(()),
        }
    }

    /// 应用启动：从配置条目装载插件注册表（装载失败进错误态，离线可用），
    /// 并补装配置中缺失的内置插件——与其余来源同一安装管线（落位 + 写条目）。
    pub fn startup(&self, store: &AppStore) {
        // 从配置文件读取插件条目（短锁：读取后立即释放，后续安装须重新加锁）。
        let plugin_entries = {
            let store_guard = match store.read() {
                Ok(guard) => guard,
                Err(_) => {
                    eprintln!("failed to lock store during plugin startup");
                    return;
                }
            };
            match store_guard.list_plugins() {
                Ok(plugin_entries) => plugin_entries,
                Err(_) => {
                    eprintln!("failed to list plugins from store during plugin startup");
                    return;
                }
            }
        };
        // 将插件装载进内存
        let mut plugins = HashMap::with_capacity(plugin_entries.len());
        for entry in &plugin_entries {
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
                Ok(mut loaded_plugin) => {
                    if let Err(err) = self.expand_config_dir(&mut loaded_plugin.manifest) {
                        plugins.insert(plugin_id, PluginState::Error(err));
                        continue;
                    }
                    if entry.enabled {
                        PluginState::Loaded(Arc::new(loaded_plugin))
                    } else {
                        PluginState::Disabled(loaded_plugin.manifest)
                    }
                }
                Err(err) => PluginState::Error(err),
            };
            plugins.insert(plugin_id, plugin_state);
        }

        if let Err(err) = self.entries.write().map(|mut entries| *entries = plugins) {
            eprintln!("failed to lock plugin registry during startup: {err}");
            return;
        }

        // 筛选出配置中缺失的内置插件：随后的安装管线会为其落位并补写条目。
        let missing_builtins: Vec<&str> = [(BUILTIN_PI_SOURCE, BUILTIN_PI_ID)]
            .iter()
            .filter(|(_, plugin_id)| !plugin_entries.iter().any(|entry| entry.id == *plugin_id))
            .map(|(plugin_source, _)| *plugin_source)
            .collect();

        for builtin_source in missing_builtins {
            if let Err(err) = self.add_plugin(store, builtin_source) {
                eprintln!("failed to install builtin plugin {builtin_source}: {err}");
            }
        }
    }

    /// 当前注册表视图。
    pub fn list(&self, store: &AppStore) -> Result<Vec<PluginView>, String> {
        let plugin_entries = store.read()?.list_plugins().map_err(|_| "配置文件损坏")?;
        let plugins = self.entries.read().map_err(|_| "插件注册表不可用")?;
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
                        PluginState::Disabled(manifest) => plugin_view(entry, Some(manifest), None),
                        PluginState::Error(err) => plugin_view(entry, None, Some(err.to_owned())),
                    },
                    None => plugin_view(entry, None, Some(format!("插件 {} 未加载", entry.id))),
                }
            })
            .collect())
    }

    /// 添加插件（内置、https 与本机路径来源同一流程）：
    /// 1. 先检查来源是否重复。
    /// 2. 随后获取 manifest 与 wasm、id 冲突检查、链接期校验，全部通过才落位。
    /// 3. 安装成功后写入配置条目；此前任何一步失败都不写条目、不留落位残留。
    ///
    /// 链接期校验（`LoadedPlugin::validate`）只做字节反序列化与导入/导出类型匹配，
    /// 不触发组件 init、不预开放真实配置目录。实投影阶段才完整实例化（见
    /// `write_providers`）。
    ///
    /// 整个流程在 `install_lock` 内独占：避免两个并发 install 各自落位后又被对方
    /// 的清理逻辑误删赢家的产物。配置存储只在短读/短写处加锁：获取、校验与落位
    /// 全程不持锁。
    pub fn add_plugin(&self, store: &AppStore, source: &str) -> Result<(), String> {
        let _install_guard = self.install_lock.lock().map_err(|_| "插件安装锁不可用")?;

        // 来源形态在发起获取之前判定：无法识别的来源直接拒绝。
        if SourceKind::from_source(source).is_none() {
            return Err(format!(
                "无法识别的插件来源：{source}（支持内置 builtin:<id>、指向 manifest.json 的 https 地址或本机绝对路径）"
            ));
        }

        // 1. 来源即条目唯一身份：重复添加在发起下载之前拒绝。
        let duplicate = store
            .read()?
            .list_plugins()?
            .iter()
            .any(|plugin| plugin.source == source);
        if duplicate {
            return Err(StoreError::DuplicateSource {
                source: source.to_owned(),
            }
            .into());
        }

        // 2. 获取 manifest 与 wasm、校验并落位；获取与校验全程不持配置存储锁。
        let mut loaded = self.fetch_placed(source)?.load()?;
        self.check_id_conflict(store, source, &loaded.manifest.id)?;
        self.expand_config_dir(&mut loaded.manifest)?;
        // 链接期校验先于落位：wasm 不是组件或接口不兼容时不落位、不写条目。
        // 仅做字节反序列化与类型检查，不触发组件 init、不预开放真实配置目录；
        // 避免未授权的 init 代码在安装失败时仍写入 plugin config_dir。
        loaded.validate(&self.engine)?;
        loaded.plugin.save()?;

        // 3. 安装成功后写入条目：`id` 是来源到落位目录的唯一映射（见 ADR 0006）。
        //    写入临界区内复检 id 冲突：检查（check_id_conflict）与写入之间，
        //    另一来源可能已占用同一 id。
        let entry = PluginEntry {
            source: source.to_owned(),
            enabled: true,
            id: loaded.manifest.id.clone(),
        };
        if let Err(e) = store.write()?.add_plugin(&entry) {
            // 条目写不进去，就不能留下注册表看不见的落位目录。
            // 清理失败时把残留路径一并回报：避免注释承诺与实际行为偏离。
            if let Err(uninstall_err) = loaded.plugin.uninstall() {
                return Err(format!(
                    "{}；落位目录清理失败：{}（残留路径：{}）",
                    String::from(&e),
                    uninstall_err,
                    loaded.plugin.path().display()
                ));
            }
            return Err(e.into());
        }
        self.entries
            .write()
            .map_err(|_| "插件注册表不可用")?
            .insert(
                loaded.manifest.id.clone(),
                PluginState::Loaded(Arc::new(loaded)),
            );
        Ok(())
    }

    /// 从来源处获取 manifest 与 wasm，构造待落位的插件。
    ///
    /// 名为「获取」而非「下载」：https 来源走网络，file 来源读本机文件，
    /// 内置来源直接取内嵌字节（见 ADR 0004）。
    fn fetch_placed(&self, source: &str) -> Result<PlacedPlugin, String> {
        let source_kind = SourceKind::from_source(source).ok_or("unknown source kind")?;

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
    /// 禁用：先改配置条目，再把内存注册表换成 Disabled 态（保留 manifest 供列表展示）。
    /// 启用：先从落位目录重新装载并展开 config_dir，装载成功才写配置条目并同步注册表
    /// ——装载失败即报错且不改配置，修复落位文件后重新启用即可。
    /// 两次加锁之间条目可能被移除：写配置前按来源复检，条目已消失即报错。
    pub fn set_enabled(&self, store: &AppStore, source: &str, enabled: bool) -> Result<(), String> {
        let entry = store.read()?.plugin_by_source(source)?;
        if entry.enabled == enabled {
            return Ok(());
        }

        // 启用先装载再写配置：装载失败不写条目、不动注册表，旧状态保持可用。
        let loaded = if enabled {
            let mut loaded =
                PlacedPlugin::from_plugin_dir(&self.maestro_paths.plugin_dir(&entry.id))?.load()?;
            self.expand_config_dir(&mut loaded.manifest)?;
            Some(loaded)
        } else {
            None
        };

        let entry = {
            let mut guard = store.write()?;
            // 复检：装载期间条目可能已被移除，不得按过期条目继续写。
            let entry = guard.plugin_by_source(source)?;
            if entry.enabled == enabled {
                // 并发方已完成同样的切换：配置与注册表均已同步，直接成功。
                return Ok(());
            }
            guard.set_plugin_enabled(source, enabled)?;
            entry
        };

        let mut entries = self.entries.write().map_err(|_| "插件注册表不可用")?;
        match loaded {
            Some(loaded) => {
                entries.insert(entry.id.clone(), PluginState::Loaded(Arc::new(loaded)));
            }
            None => {
                if let Some(state) = entries.get_mut(&entry.id)
                    && let PluginState::Loaded(loaded) = state
                {
                    let manifest = loaded.manifest.clone();
                    *state = PluginState::Disabled(manifest);
                }
            }
        }
        Ok(())
    }

    /// 移除插件：先从内存注册表卸载，再删配置条目，最后删落位目录；
    /// 条目已不存在同样成功（幂等）。
    ///
    /// 只删宿主落位的副本，不动用户的插件项目目录；内置插件不可移除（随应用
    /// 分发，见 ADR 0004）。整个流程在 `install_lock` 内独占：读条目、卸载内存、
    /// 删条目与删落位目录必须连续完成，中间不得插入另一次 install——安装先落位
    /// 后写条目，删目录若放到锁外，并发安装可能在条目删除后重新落位而被误删。
    pub fn remove_plugin(&self, store: &AppStore, source: &str) -> Result<(), String> {
        let _install_guard = self.install_lock.lock().map_err(|_| "插件安装锁不可用")?;

        // 内置插件随应用分发，不可移除（见 ADR 0004）。
        if matches!(SourceKind::from_source(source), Some(SourceKind::Builtin)) {
            return Err(format!("内置插件不可移除：{source}"));
        }

        let mut guard = store.write()?;
        let entry = match guard.plugin_by_source(source) {
            Ok(entry) => entry,
            // 条目已不存在：上一次移除已连同落位目录一并删除，幂等成功。
            Err(StoreError::MissingSource { .. }) => return Ok(()),
            Err(e) => return Err(e.into()),
        };

        // 1. 先从内存注册表卸载：此后删除期间插件不再可执行。
        self.entries
            .write()
            .map_err(|_| "插件注册表不可用")?
            .remove(&entry.id);

        // 2. 删配置条目（锁内读-改-写，与读条目同处一个临界区）。
        guard.delete_plugin(source)?;

        // 3. 删落位目录：只删宿主副本，不动用户的插件项目目录。
        remove_placed_dir(&self.maestro_paths.plugin_dir(&entry.id))
    }

    /// 对注册表中的每个插件执行投影，返回逐插件报告：
    /// Loaded 实例化组件后调用 `write_provider`；Disabled 与 Error 态不执行投影。
    /// 单个插件的失败不影响其它插件，失败原因写入该插件的报告。
    ///
    /// 插件执行时长不受应用控制：注册表锁只在快照阶段短暂持有（克隆 Arc 与
    /// 收集跳过态），wasm 实例化与调用全部在锁外进行，不阻塞其它命令。
    pub fn write_providers(
        &self,
        providers: &BTreeMap<String, Provider>,
    ) -> Result<Vec<PluginApplyReport>, String> {
        let (mut reports, loaded) = {
            let entries = self.entries.read().map_err(|_| "插件注册表不可用")?;
            let mut reports = Vec::with_capacity(entries.len());
            let mut loaded = Vec::new();
            for (id, state) in entries.iter() {
                match state {
                    PluginState::Loaded(loaded_plugin) => {
                        loaded.push((id.clone(), Arc::clone(loaded_plugin)));
                    }
                    PluginState::Disabled(_) => reports.push(PluginApplyReport {
                        id: id.clone(),
                        status: "skipped",
                        files: Vec::new(),
                        skipped: Vec::new(),
                        reason: Some("插件已禁用".to_owned()),
                    }),
                    PluginState::Error(reason) => reports.push(PluginApplyReport {
                        id: id.clone(),
                        status: "skipped",
                        files: Vec::new(),
                        skipped: Vec::new(),
                        reason: Some(reason.clone()),
                    }),
                }
            }
            (reports, loaded)
        };

        for (id, loaded_plugin) in loaded {
            let report = match loaded_plugin
                .instantiate_component(&self.engine)
                .and_then(|mut instantiated| instantiated.write_provider(providers))
            {
                Ok((files, skipped)) => PluginApplyReport {
                    id,
                    status: "applied",
                    files,
                    skipped,
                    reason: None,
                },
                Err(reason) => PluginApplyReport {
                    id,
                    status: "failed",
                    files: Vec::new(),
                    skipped: Vec::new(),
                    reason: Some(reason),
                },
            };
            reports.push(report);
        }
        Ok(reports)
    }

    /// 重新加载：「按配置中的来源」无条件重新获取 manifest 与 wasm，成功才替换落位
    /// 目录——失败时旧版本保持可用。每次只作用于一个来源（见 ADR 0006）。
    ///
    /// 内置来源的“重新获取”即从内嵌字节重新构造落位副本，用于恢复被改动或
    /// 损坏的宿主副本（无上游新版本可言，但 ID 一致性校验仍执行）。
    ///
    /// 整个流程在 `install_lock` 内独占：与 add / remove 共用同一把锁，避免
    /// reload 写到一半时另一 add 把同一 id 的产物覆盖或被 remove 误删。
    pub fn reload_plugin(&self, store: &AppStore, source: &str) -> Result<(), String> {
        let _install_guard = self.install_lock.lock().map_err(|_| "插件安装锁不可用")?;

        // 条目是来源到落位目录的唯一映射：未知来源无从重新装载。
        let entry = store.read()?.plugin_by_source(source)?;

        // 重新获取 manifest 与 wasm、校验：全程不持配置存储锁（与添加同构）。
        let mut loaded = self.fetch_placed(source)?.load()?;
        // 上游 id 变更报错并保持旧状态：落位目录由条目 id 定位，id 漂移会架空映射。
        if loaded.manifest.id != entry.id {
            return Err(format!(
                "上游 manifest 的插件 id 已从「{}」变更为「{}」，拒绝重新加载",
                entry.id, loaded.manifest.id
            ));
        }
        self.expand_config_dir(&mut loaded.manifest)?;
        // 链接期校验先于替换：wasm 不是组件或接口不兼容时旧版本保持可用。
        // 不触发组件 init、不预开放真实配置目录，避免未授权写入。
        loaded.validate(&self.engine)?;
        // 替换落位目录：swap_placed 先备份旧版本，替换失败即恢复，旧版本保持可用。
        loaded.plugin.save()?;

        // 条目（id 与 enabled）不变，仅把内存注册表同步为新装载的版本。
        let state = if entry.enabled {
            PluginState::Loaded(Arc::new(loaded))
        } else {
            PluginState::Disabled(loaded.manifest)
        };
        self.entries
            .write()
            .map_err(|_| "插件注册表不可用")?
            .insert(entry.id, state);
        Ok(())
    }

    /// id 冲突检查：同一 id 只能由一个来源持有。
    fn check_id_conflict(&self, store: &AppStore, source: &str, id: &str) -> Result<(), String> {
        let guard = store.read()?;
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

    /// 把 manifest 的 `config_dir` 展开为宿主内的绝对路径，原位写回 manifest：
    /// 投影按该路径预开放目录。展开含目录创建与逃逸校验（见 `resolve_config_dir`）。
    fn expand_config_dir(&self, manifest: &mut Manifest) -> Result<(), String> {
        manifest.config_dir = self
            .resolve_config_dir(&manifest.config_dir)?
            .display()
            .to_string();
        Ok(())
    }
}

/// 测试共享助手：临时主目录 + 替身 fetcher 的插件服务与配置存储。
#[cfg(test)]
pub(crate) mod testutil {
    use std::{
        path::Path,
        sync::{Arc, RwLock},
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
            store: RwLock::new(Store::new(&maestro_paths)),
        }
    }

    /// 测试读取配置快照：走与服务同一套短锁访问。
    pub(crate) fn snapshot(store: &AppStore) -> Config {
        store.read().unwrap().get().unwrap().clone()
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
        ] {
            let err = service.add_plugin(&store, source).unwrap_err();
            assert!(
                err.contains("无法识别的插件来源"),
                "{source} 应被拒绝：{err}"
            );
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

    /// 同 id、不同来源的并发 install：install_lock 串行化整个流程，
    /// 恰好一个成功、另一个失败，赢家的落位目录必须保留。
    /// 修复前的 race:两个线程都通过预检、各自 save 到同一 plugin_dir(id)、
    /// 后写入 store.add_plugin 的线程报错后调用 uninstall 误删赢家的文件。
    #[test]
    fn concurrent_add_plugin_with_same_id_serializes_and_preserves_winner() {
        use std::thread;

        let home = temp_home();
        let maestro_paths = MaestroPaths::new(home.path());

        let source_a = "https://example.com/a/manifest.json";
        let source_b = "https://example.com/b/manifest.json";
        let wasm_a = "https://example.com/a/plugin.wasm";
        let wasm_b = "https://example.com/b/plugin.wasm";
        let fetcher = StubFetcher::new();
        fetcher.serve(source_a, manifest_json("pi", "Pi-A", "pi", "~/.pi", wasm_a));
        fetcher.serve(source_b, manifest_json("pi", "Pi-B", "pi", "~/.pi", wasm_b));
        fetcher.serve(wasm_a, builtin::PI_WASM.to_vec());
        fetcher.serve(wasm_b, builtin::PI_WASM.to_vec());

        let store = Arc::new(store_at(home.path()));
        let service = Arc::new(stub_service(home.path(), fetcher));

        let handles = vec![
            {
                let store = Arc::clone(&store);
                let service = Arc::clone(&service);
                thread::spawn(move || service.add_plugin(&store, source_a))
            },
            {
                let store = Arc::clone(&store);
                let service = Arc::clone(&service);
                thread::spawn(move || service.add_plugin(&store, source_b))
            },
        ];
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        let successes = results.iter().filter(|r| r.is_ok()).count();
        let failures = results.iter().filter(|r| r.is_err()).count();
        assert_eq!(successes, 1, "恰好一个 install 成功：{results:?}");
        assert_eq!(failures, 1, "另一个 install 失败：{results:?}");

        // 配置文件中恰好一条记录，赢家独享。
        let plugins = snapshot(&store).plugins;
        assert_eq!(plugins.len(), 1, "只有一个 id 落盘");
        assert_eq!(plugins[0].id, "pi", "赢家的 id 落盘");

        // 赢家的落位目录与产物必须保留——这是修复的核心不变量。
        let placed_dir = maestro_paths.plugin_dir("pi");
        assert!(
            placed_dir.join(PLACED_MANIFEST).exists(),
            "赢家的 manifest.json 必须保留"
        );
        assert!(
            placed_dir.join(PLACED_WASM).exists(),
            "赢家的 plugin.wasm 必须保留"
        );
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

    #[test]
    fn remove_plugin_unloads_memory_deletes_entry_then_dir() {
        let home = temp_home();
        let maestro_paths = MaestroPaths::new(home.path());
        let store = store_at(home.path());
        let service = stub_service(home.path(), https_stub("pi2", "Pi2"));
        service.add_plugin(&store, MANIFEST_URL).unwrap();

        service.remove_plugin(&store, MANIFEST_URL).unwrap();

        assert!(snapshot(&store).plugins.is_empty(), "配置条目已删除");
        assert!(!maestro_paths.plugin_dir("pi2").exists(), "落位目录已删除");
        assert!(
            service.entries.read().unwrap().is_empty(),
            "内存注册表已卸载"
        );
        assert!(service.list(&store).unwrap().is_empty());
    }

    #[test]
    fn remove_plugin_is_idempotent_when_the_entry_is_already_gone() {
        let home = temp_home();
        let store = store_at(home.path());
        let service = test_service(home.path());

        service
            .remove_plugin(&store, MANIFEST_URL)
            .expect("条目不存在同样成功");
        service
            .remove_plugin(&store, MANIFEST_URL)
            .expect("移除两次同样成功");
    }

    #[test]
    fn remove_plugin_rejects_builtin_source() {
        let home = temp_home();
        let store = store_at(home.path());
        let service = test_service(home.path());

        let err = service
            .remove_plugin(&store, builtin::BUILTIN_PI_SOURCE)
            .unwrap_err();

        assert!(err.contains("内置插件不可移除"), "{err}");
    }

    #[test]
    fn remove_plugin_keeps_the_user_project_dir_for_file_sources() {
        let home = temp_home();
        let maestro_paths = MaestroPaths::new(home.path());
        let store = store_at(home.path());
        let service = test_service(home.path());
        let entry = "target/wasm32-wasip2/release/maestro_plugin_pi.wasm";
        let (source, manifest) = source_dir("pi2", entry);
        let source_id = manifest.display().to_string();
        service.add_plugin(&store, &source_id).unwrap();

        service.remove_plugin(&store, &source_id).unwrap();

        assert!(snapshot(&store).plugins.is_empty());
        assert!(!maestro_paths.plugin_dir("pi2").exists());
        assert!(
            source.path().join(entry).exists(),
            "只删落位副本，不动用户的插件项目目录"
        );
    }

    #[test]
    fn reload_plugin_refetches_and_replaces_the_placed_version() {
        let home = temp_home();
        let maestro_paths = MaestroPaths::new(home.path());
        let store = store_at(home.path());
        let fetcher = Arc::new(https_stub("pi2", "Pi2"));
        let service = PluginService::with_fetcher(&MaestroPaths::new(home.path()), fetcher.clone());
        service.add_plugin(&store, MANIFEST_URL).unwrap();

        // 上游改版：同 id、新名字。
        fetcher.serve(
            MANIFEST_URL,
            manifest_json("pi2", "Pi3", "pi", "~/.pi2", WASM_URL),
        );

        service.reload_plugin(&store, MANIFEST_URL).unwrap();

        let dir = maestro_paths.plugin_dir("pi2");
        assert_eq!(
            fs::read_to_string(dir.join(PLACED_MANIFEST)).unwrap(),
            manifest_json("pi2", "Pi3", "pi", "~/.pi2", WASM_URL),
            "落位目录已替换为新版本"
        );
        let view = &service.list(&store).unwrap()[0];
        assert_eq!(view.error, None);
        assert_eq!(view.name.as_deref(), Some("Pi3"), "注册表同步新 manifest");
    }

    #[test]
    fn reload_failure_keeps_the_old_placed_version_usable() {
        let home = temp_home();
        let maestro_paths = MaestroPaths::new(home.path());
        let store = store_at(home.path());
        let fetcher = Arc::new(https_stub("pi2", "Pi2"));
        let service = PluginService::with_fetcher(&MaestroPaths::new(home.path()), fetcher.clone());
        service.add_plugin(&store, MANIFEST_URL).unwrap();
        fetcher.fail(MANIFEST_URL, "网络不可达");

        let err = service.reload_plugin(&store, MANIFEST_URL).unwrap_err();

        assert!(err.contains("网络不可达"), "{err}");
        let dir = maestro_paths.plugin_dir("pi2");
        assert_eq!(
            fs::read_to_string(dir.join(PLACED_MANIFEST)).unwrap(),
            manifest_json("pi2", "Pi2", "pi", "~/.pi2", WASM_URL),
            "获取失败时旧版本保持可用"
        );
        let view = &service.list(&store).unwrap()[0];
        assert_eq!(view.error, None);
        assert_eq!(view.name.as_deref(), Some("Pi2"), "注册表保持旧版本");
    }

    #[test]
    fn reload_plugin_rejects_upstream_id_change_and_keeps_old_state() {
        let home = temp_home();
        let maestro_paths = MaestroPaths::new(home.path());
        let store = store_at(home.path());
        let fetcher = Arc::new(https_stub("pi2", "Pi2"));
        let service = PluginService::with_fetcher(&MaestroPaths::new(home.path()), fetcher.clone());
        service.add_plugin(&store, MANIFEST_URL).unwrap();

        // 上游把 id 改成了 pi3：落位目录由条目 id 定位，id 漂移会架空映射（见 ADR 0006）。
        fetcher.serve(
            MANIFEST_URL,
            manifest_json("pi3", "Pi2", "pi", "~/.pi2", WASM_URL),
        );

        let err = service.reload_plugin(&store, MANIFEST_URL).unwrap_err();

        assert!(
            err.contains("pi2") && err.contains("pi3") && err.contains("拒绝重新加载"),
            "{err}"
        );
        assert_eq!(
            snapshot(&store).plugins[0].id,
            "pi2",
            "拒绝时不得改配置条目"
        );
        let dir = maestro_paths.plugin_dir("pi2");
        assert_eq!(
            fs::read_to_string(dir.join(PLACED_MANIFEST)).unwrap(),
            manifest_json("pi2", "Pi2", "pi", "~/.pi2", WASM_URL),
            "拒绝时旧版本保持可用"
        );
        assert_eq!(
            service.list(&store).unwrap()[0].name.as_deref(),
            Some("Pi2")
        );
        assert!(
            !maestro_paths.plugin_dir("pi3").exists(),
            "拒绝时不得落位新目录"
        );
    }

    #[test]
    fn reload_plugin_keeps_the_disabled_state_with_a_fresh_manifest() {
        let home = temp_home();
        let maestro_paths = MaestroPaths::new(home.path());
        let store = store_at(home.path());
        let fetcher = Arc::new(https_stub("pi2", "Pi2"));
        let service = PluginService::with_fetcher(&MaestroPaths::new(home.path()), fetcher.clone());
        service.add_plugin(&store, MANIFEST_URL).unwrap();
        service.set_enabled(&store, MANIFEST_URL, false).unwrap();

        fetcher.serve(
            MANIFEST_URL,
            manifest_json("pi2", "Pi3", "pi", "~/.pi2", WASM_URL),
        );
        service.reload_plugin(&store, MANIFEST_URL).unwrap();

        assert!(
            !snapshot(&store).plugins[0].enabled,
            "重新加载不改条目的 enabled"
        );
        let dir = maestro_paths.plugin_dir("pi2");
        assert_eq!(
            fs::read_to_string(dir.join(PLACED_MANIFEST)).unwrap(),
            manifest_json("pi2", "Pi3", "pi", "~/.pi2", WASM_URL)
        );
        let view = &service.list(&store).unwrap()[0];
        assert!(!view.enabled);
        assert_eq!(view.error, None);
        assert_eq!(
            view.name.as_deref(),
            Some("Pi3"),
            "禁用态仍以新 manifest 供展示"
        );
    }

    #[test]
    fn reload_plugin_rejects_unknown_sources() {
        let home = temp_home();
        let store = store_at(home.path());
        let service = test_service(home.path());

        let err = service.reload_plugin(&store, MANIFEST_URL).unwrap_err();
        assert!(err.contains("插件条目不存在"), "{err}");
    }

    /// 内置插件的「重新加载」按内嵌字节重建落位副本：损坏的宿主副本可被恢复，
    /// 配置条目（`enabled` 等）保持不变。
    #[test]
    fn reload_builtin_plugin_restores_a_corrupted_placed_copy() {
        let home = temp_home();
        let maestro_paths = MaestroPaths::new(home.path());
        let store = store_at(home.path());
        let service = test_service(home.path());

        service.startup(&store);
        assert_eq!(snapshot(&store).plugins.len(), 1);

        // 故意损坏宿主落位的 wasm：重新加载必须按内嵌字节还原。
        let placed_wasm = maestro_paths.plugin_dir("pi").join(PLACED_WASM);
        fs::write(&placed_wasm, b"corrupted").unwrap();

        service
            .reload_plugin(&store, builtin::BUILTIN_PI_SOURCE)
            .unwrap();

        assert_eq!(
            fs::read(&placed_wasm).unwrap(),
            builtin::PI_WASM.to_vec(),
            "宿主副本已按内嵌字节恢复"
        );
        let view = &service.list(&store).unwrap()[0];
        assert_eq!(view.error, None);
        assert_eq!(view.name.as_deref(), Some("Pi"), "注册表重新装载成功");
        assert!(snapshot(&store).plugins[0].enabled, "配置条目未被改动");
    }

    /// 全新主目录：内置插件与其余来源同一安装管线（落位 + 写条目），且可重复启动。
    #[test]
    fn startup_installs_missing_builtin_plugin_like_any_other_source() {
        let home = temp_home();
        let maestro_paths = MaestroPaths::new(home.path());
        let store = store_at(home.path());
        let service = test_service(home.path());

        service.startup(&store);

        let plugins = snapshot(&store).plugins;
        assert_eq!(plugins.len(), 1, "启动时补装缺失的内置插件");
        assert_eq!(plugins[0].source, builtin::BUILTIN_PI_SOURCE);
        assert_eq!(plugins[0].id, builtin::BUILTIN_PI_ID);
        assert!(plugins[0].enabled);
        assert!(
            maestro_paths
                .plugin_dir("pi")
                .join(PLACED_MANIFEST)
                .exists()
                && maestro_paths.plugin_dir("pi").join(PLACED_WASM).exists(),
            "内置插件与其余来源一样落位到磁盘"
        );

        let views = service.list(&store).unwrap();
        assert_eq!(views.len(), 1);
        assert!(views[0].builtin);
        assert_eq!(views[0].error, None, "{:?}", views[0].error);

        // 再次启动：条目已存在，不重复安装，注册表照常重建。
        service.startup(&store);
        assert_eq!(snapshot(&store).plugins.len(), 1, "重复启动不产生重复条目");
        assert_eq!(service.list(&store).unwrap()[0].error, None);
    }

    /// 逐插件报告：applied / failed / skipped 分类，单插件失败不影响其它插件。
    #[test]
    fn write_providers_reports_per_plugin_and_isolates_failures() {
        use crate::provider::Endpoints;

        let home = temp_home();
        let store = store_at(home.path());
        let fetcher = https_stub("pi2", "Pi2");
        fetcher.serve(
            "https://example.com/pi3/manifest.json",
            manifest_json("pi3", "Pi3", "pi", "~/.pi3", WASM_URL),
        );
        let service = stub_service(home.path(), fetcher);

        service.add_plugin(&store, MANIFEST_URL).unwrap();
        service
            .add_plugin(&store, "https://example.com/pi3/manifest.json")
            .unwrap();
        service
            .set_enabled(&store, "https://example.com/pi3/manifest.json", false)
            .unwrap();

        // 直接注入注册表的两种异常态：坏 wasm 的 Loaded 与显式 Error。
        let bad = PlacedPlugin::new(
            &home.path().join("pibad"),
            manifest_json("pibad", "PiBad", "pi", "~/.pibad", "plugin.wasm"),
            b"not a wasm component",
        )
        .load()
        .unwrap();
        let mut entries = service.entries.write().unwrap();
        entries.insert("pibad".to_owned(), PluginState::Loaded(Arc::new(bad)));
        entries.insert(
            "pierr".to_owned(),
            PluginState::Error("配置已损坏".to_owned()),
        );
        drop(entries);

        let providers = BTreeMap::from([(
            "gateway".to_owned(),
            Provider {
                base_url: Endpoints {
                    openai_completions: Some("https://api.example.com/v1".to_owned()),
                    ..Endpoints::default()
                },
                ..Provider::default()
            },
        )]);

        let reports = service.write_providers(&providers).unwrap();

        let by_id: HashMap<&str, &PluginApplyReport> = reports
            .iter()
            .map(|report| (report.id.as_str(), report))
            .collect();
        assert_eq!(reports.len(), 4, "每个注册表条目一份报告");
        let applied = by_id.get("pi2").unwrap();
        assert_eq!(applied.status, "applied");
        assert_eq!(applied.files, vec!["agent/models.json"]);
        let failed = by_id.get("pibad").unwrap();
        assert_eq!(failed.status, "failed");
        assert!(
            failed
                .reason
                .as_deref()
                .unwrap_or_default()
                .contains("不是有效的 WASM 组件"),
            "{:?}",
            failed.reason
        );
        assert_eq!(by_id.get("pi3").unwrap().status, "skipped", "禁用态跳过");
        assert_eq!(
            by_id.get("pi3").unwrap().reason.as_deref(),
            Some("插件已禁用")
        );
        assert_eq!(by_id.get("pierr").unwrap().status, "skipped", "错误态跳过");
        assert_eq!(
            by_id.get("pierr").unwrap().reason.as_deref(),
            Some("配置已损坏")
        );
    }

    /// 替换落位目录失败时，旧目录必须从备份恢复原位。
    #[test]
    fn swap_placed_restores_the_backup_when_replacement_fails() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("pi");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("marker"), "old").unwrap();

        // staging 不存在：替换必然失败，走备份恢复路径。
        let err = swap_placed(&dir.path().join("missing-staging"), &target).unwrap_err();

        assert!(err.contains("替换插件目录失败"), "{err}");
        assert_eq!(
            fs::read_to_string(target.join("marker")).unwrap(),
            "old",
            "替换失败后旧版本恢复原位"
        );
        assert!(
            !target.with_extension("old").exists(),
            "备份已恢复回目标位置，不残留备份目录"
        );
    }
}
