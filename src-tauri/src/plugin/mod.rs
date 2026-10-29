//! 插件服务：从配置条目装载插件（内置字节或落位目录）、维护内存注册表、
//! 并把 Provider 投影进各插件声明的配置目录（wasmtime 宿主，WASI 0.2）。
//!
//! 架构决策见 ADR 0004：宿主不代写文件，而是把 manifest 声明的 `config_dir`
//! 预开放给组件（guest 路径 `/`），插件在沙箱内经 WASI 直接落盘。
//! 第三方来源（https 与 file）的获取、落位与生命周期见 ADR 0006。
//!
//! 本模块持有插件服务的共享词汇：四层类型（`PlacedPlugin` → `PluginMeta` →
//! `LoadedPlugin` → `PluginState`）、落位目录约定，以及插件配置类型。前几者是
//! 模块私有项——定义在模块根，子模块可直接使用（Rust 的私有项对后代模块可见），
//! 不向 crate 外暴露。插件条目不是本模块的词汇：它是配置文件的形状，住在
//! crate 根的中立层 [`crate::config`]（见 [`ports`]）。
//!
//! 行为按职责拆在同级子模块：
//! - [`install`]：获取、落位、启停、移除与重新加载；
//! - [`projection`]：Provider 投影与文件路径拼接；
//! - [`secrets`]：投影前解析出的密钥（slug → 明文 / 未设置）；
//! - [`config`]：插件配置（`config.json`）读写；
//! - [`source`]：插件来源与 https 地址的判定；
//! - [`manifest`]：manifest 的解析、校验与 `config_dir` 声明语法；
//! - [`ports`]：服务层对配置存储的端口（它真正需要哪些能力）。

mod builtin;
mod config;
mod execution;
mod fetch;
mod install;
mod manifest;
pub(crate) mod ports;
mod projection;
mod secrets;
mod source;

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) mod testutil;

use std::{
    collections::{BTreeMap, HashMap},
    fmt::Display,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, RwLock},
};

use serde::{Deserialize, Serialize};
use wasmtime::Engine;

use crate::{
    config::PluginEntry,
    errors::PluginError,
    fs_util::{Durability, write_atomic},
    paths::Paths,
    plugin::builtin::BUILTINS,
    variables::Variables,
};
pub(crate) use execution::SkippedProvider;
use fetch::{Fetcher, HttpFetcher};
use manifest::{Manifest, parse_placed};
use ports::PluginEntries;
use source::is_builtin_source;

/// 落位目录中的 manifest 文件名。
const PLACED_MANIFEST: &str = "manifest.json";
/// 落位目录中的 wasm 文件名；上游资源内容固定落到该名。装载时 `entry` 仍会被
/// 反序列化，但其指向的路径不再被解析——落位产物只认这个固定文件名。
const PLACED_WASM: &str = "plugin.wasm";
/// 落位目录中的插件配置文件名（ADR 0018）：与两个宿主产物混居，逐文件覆盖的
/// 落位替换与重载完全不碰它。
const PLACED_CONFIG: &str = "config.json";

struct PlacedPlugin {
    plugin_dir: PathBuf,
    raw_manifest: String,
    /// wasm 字节：与装载后的插件共用同一份（Arc 克隆），不复制整份字节。
    wasm: Arc<[u8]>,
}

/// 插件描述：manifest 声明 + 解析后的配置目录。
///
/// 两者分开持有：manifest 是反序列化的产物（`config_dir` 只是**声明**），
/// 配置目录是显式解析一步的产物（宿主绝对路径，见 ADR 0022）。禁用态也保留
/// 这份描述——列表要展示配置目录。
#[derive(Clone)]
struct PluginMeta {
    manifest: Manifest,
    config_dir: PathBuf,
}

struct LoadedPlugin {
    meta: PluginMeta,
    wasm: Arc<[u8]>,
    plugin: PlacedPlugin,
}

impl PlacedPlugin {
    fn new(plugin_dir: &Path, raw_manifest: String, wasm: &[u8]) -> Self {
        Self {
            plugin_dir: plugin_dir.to_path_buf(),
            raw_manifest,
            wasm: Arc::from(wasm),
        }
    }

    fn from_plugin_dir(plugin_dir: &Path) -> Result<Self, PluginError> {
        let manifest_path = plugin_dir.join(PLACED_MANIFEST);
        let raw_manifest = fs::read_to_string(&manifest_path)
            .map_err(|e| PluginError::read_file(&manifest_path, e.to_string()))?;

        let wasm_path = plugin_dir.join(PLACED_WASM);
        let wasm =
            fs::read(&wasm_path).map_err(|e| PluginError::read_file(&wasm_path, e.to_string()))?;

        Ok(Self::new(plugin_dir, raw_manifest, wasm.as_slice()))
    }

    /// 落位目录的绝对路径（暴露给错误信息，便于用户手动清理）。
    fn path(&self) -> &Path {
        &self.plugin_dir
    }

    fn save(&self) -> Result<(), PluginError> {
        // 首次安装即建目录；已存在则逐文件覆盖，目录中宿主产物以外的文件
        // （如插件配置，见 ADR 0018）完全不受影响。
        fs::create_dir_all(&self.plugin_dir).map_err(|e| PluginError::CreateDirectory {
            detail: e.to_string(),
        })?;
        // 先 manifest、后 wasm：写 wasm 失败会停在「新 manifest + 旧 wasm」的
        // 混合态，恢复手段是从来源重新加载（备份回滚的既有保证已取消，见 ADR 0018）。
        write_placed_file(
            &self.plugin_dir,
            PLACED_MANIFEST,
            self.raw_manifest.as_bytes(),
        )?;
        write_placed_file(&self.plugin_dir, PLACED_WASM, self.wasm.as_ref())?;
        log::debug!("placed plugin: {}", self.plugin_dir.display());
        Ok(())
    }

    fn uninstall(&self) -> Result<(), PluginError> {
        remove_placed_dir(&self.plugin_dir)
    }

    /// 装载：解析 manifest 并解析 `config_dir`（见 ADR 0011）。
    ///
    /// 装载是**显式**的一步：解析（反序列化）只产出声明、不碰文件系统，落位
    /// 目录重建时由此把声明解析成宿主绝对路径并建出目录（见 ADR 0022）。
    fn load(self, paths: &Paths) -> Result<LoadedPlugin, PluginError> {
        let manifest = parse_placed(self.raw_manifest.as_str())?;
        let config_dir = manifest.config_dir.resolve(paths)?;
        Ok(LoadedPlugin::from_parts(manifest, config_dir, self))
    }
}

impl LoadedPlugin {
    /// 由已解析的 manifest 与落位产物装配：配置目录必须已解析（显式一步的产物）。
    ///
    /// 安装路径上，manifest 只解析一次：来源侧解析出的 manifest 直接交给本
    /// 构造器，不再由落位产物重新解析一遍。
    fn from_parts(manifest: Manifest, config_dir: PathBuf, plugin: PlacedPlugin) -> Self {
        // wasm 字节与落位产物共用同一份：装载不再复制整份字节。
        let wasm = Arc::clone(&plugin.wasm);
        Self {
            meta: PluginMeta {
                manifest,
                config_dir,
            },
            wasm,
            plugin,
        }
    }

    /// 插件 id（落位目录名、id 冲突检查与投影报告的键）。
    fn id(&self) -> &str {
        &self.meta.manifest.id
    }
}

/// 落位目录内逐文件覆盖一个文件：同目录临时文件 + 原子改名（tmp + rename）。
/// 落位产物（manifest/wasm）与插件配置共用这一条原子写路径。
///
/// 原子写的实现收在 crate 级 [`crate::fs_util`]：宿主侧另有一处同样的
/// 「tmp + rename」（配置存储），两份实现曾各自漂移过一次。耐久性策略在这里
/// 显式声明——插件侧产物不 fsync（可从来源重新获取，见 [`Durability`]）。
fn write_placed_file(dir: &Path, name: &str, contents: &[u8]) -> Result<(), PluginError> {
    write_atomic(&dir.join(name), contents, Durability::Unsynced)
        .map_err(|e| PluginError::write_file(e.to_string()))
}

/// 删除落位目录；目录已不存在同样成功（幂等）。
fn remove_placed_dir(plugin_dir: &Path) -> Result<(), PluginError> {
    match fs::remove_dir_all(plugin_dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(PluginError::remove_directory(plugin_dir, e.to_string())),
    }
}

/// 插件的注册表描述：服务层的内部形状。
///
/// 线格式契约在命令层（见 `crate::command::plugin` 的 `PluginView`）：服务层
/// 只说「这个插件是什么」，怎么说给界面听是交付层的事（#141 待决策 A）。
#[derive(Debug)]
pub(crate) struct PluginRecord {
    pub(crate) source: String,
    /// 来源以内置前缀标识；内置插件可禁用、不可移除。
    pub(crate) builtin: bool,
    pub(crate) enabled: bool,
    pub(crate) id: String,
    /// manifest 声明并解析出的配置目录（宿主绝对路径）。
    pub(crate) config_dir: Option<PathBuf>,
    /// manifest 内联的 settings_schema（ADR 0017），界面据此渲染表单；
    /// 缺省即该插件无需用户填写。
    pub(crate) settings_schema: Option<serde_json::Value>,
    /// 装载失败的原因。
    pub(crate) error: Option<String>,
}

/// 逐插件投影的状态：已写入 / 失败 / 未执行（服务层内部形状）。
///
/// 线格式词（`applied` / `failed` / `skipped`）由命令层产出——那是交付层的
/// 契约。本枚举的英文词只服务 ADR 0008 的日志行；两者同形，由命令层的一条
/// 测试钉住。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectionStatus {
    /// 已写入：插件执行成功并写出了文件。
    Applied,
    /// 失败：插件执行了但返回错误（不写任何文件）。
    Failed,
    /// 未执行投影：插件已禁用或装载失败。
    Skipped,
}

impl Display for ProjectionStatus {
    /// 日志用的英文词（与线格式词同形，见类型文档）。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ProjectionStatus::Applied => "applied",
            ProjectionStatus::Failed => "failed",
            ProjectionStatus::Skipped => "skipped",
        })
    }
}

/// 逐插件投影的结果：状态、已写入文件、跳过的 Provider、失败原因。
///
/// 服务层内部形状；命令层把它映射成线格式的 `PluginApplyReport`。
#[derive(Debug)]
pub(crate) struct PluginOutcome {
    pub(crate) id: String,
    pub(crate) status: ProjectionStatus,
    /// 已写入文件的宿主绝对路径列表（插件返回相对 config_dir 的路径，由宿主拼接）。
    pub(crate) files: Vec<String>,
    pub(crate) skipped: Vec<SkippedProvider>,
    pub(crate) reason: Option<String>,
}

/// 逐插件状态摘要（`id:status` 逐项，空格分隔）：结果行的识别参数。
///
/// 结果行按 ADR 0008 携带逐插件状态摘要——某个插件投影失败时，日志里要能
/// 看出是哪一个。摘要由本模块产出（命令层只负责把它交给 `log_outcome`，
/// 不自己拼串）。没有插件时返回 `None`：成功行不带上下文，不留尾随的
/// 冒号与空格。
pub(crate) fn apply_summary(outcomes: &[PluginOutcome]) -> Option<String> {
    if outcomes.is_empty() {
        return None;
    }
    Some(
        outcomes
            .iter()
            .map(|outcome| format!("{}:{}", outcome.id, outcome.status))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// 插件配置（落位目录 `<id>/config.json`，ADR 0018）：该插件对全局变量的覆盖
/// 与它私有的表单数据，与插件一一对应、随插件存在。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct PluginConfig {
    /// 变量覆盖：键必须是全局变量表中已声明的变量名（写入时校验，见 `set_config`）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) variables: Variables,
    /// 表单数据：形状由 manifest 的 `settings_schema` 声明（ADR 0017），宿主不解释。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) form: Option<serde_json::Value>,
}

/// 注册表条目的装载状态；装载失败进错误态并携带人类可读原因。
enum PluginState {
    /// 损坏的插件
    Error(String),
    /// 禁用的插件，不加载到内存（保留描述供列表展示）。
    Disabled(PluginMeta),
    /// 已落位且文件没有损坏的插件，可以正常加载。
    /// Arc 便于投影前克隆快照、锁外执行 wasm（见 `write_providers`）。
    Loaded(Arc<LoadedPlugin>),
}

/// 插件服务：内存注册表由 `startup` 与各生命周期方法（安装/启停/移除/重载）
/// 按配置条目增量维护。
pub(crate) struct PluginService {
    engine: Engine,
    /// https 拉取的注入点（生产为同步 reqwest 实现，测试为替身）。
    fetcher: Arc<dyn Fetcher>,
    /// 宿主路径（构造时注入，#141 I-4）：落位目录与 `config_dir` 的平台变量
    /// 解析都从它取值，不读进程级全局。
    paths: Paths,
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
    /// 构造：注入宿主路径（`lib.rs` 在应用启动阶段完成初始化后传入，
    /// 见 `run()` 的装配，早于 logger attach）。
    pub(crate) fn new(paths: Paths) -> Self {
        Self::with_fetcher(paths, Arc::new(HttpFetcher::new()))
    }

    /// 注入 fetcher：测试用替身。
    fn with_fetcher(paths: Paths, fetcher: Arc<dyn Fetcher>) -> Self {
        Self {
            engine: build_engine(),
            fetcher,
            paths,
            entries: RwLock::new(HashMap::new()),
            install_lock: Mutex::new(()),
        }
    }

    /// 取得安装锁：安装类流程（添加/启停/移除/重载/写配置）共用同一把独占锁。
    ///
    /// 锁序 `install_lock` → store → entries：端口的每次读写各占一把短临界区，
    /// 注册表锁在其外。
    fn lock_install(&self) -> Result<MutexGuard<'_, ()>, PluginError> {
        self.install_lock
            .lock()
            .map_err(|_| PluginError::InstallLockUnavailable)
    }

    /// 应用启动：从配置条目装载插件注册表（装载失败进错误态，离线可用），
    /// 并补装配置中缺失的内置插件——与其余来源同一安装管线（落位 + 写条目）。
    ///
    /// 宿主路径由构造时注入（见 [`PluginService::new`]）。配置存储不可用则
    /// 降级——记日志、留空注册表并返回 `Ok`，由各命令各自报错。
    ///
    /// 配置存储经端口访问（见 [`ports`]）：服务层拿不到它的锁与落盘时机。
    pub(crate) fn startup(&self, store: &impl PluginEntries) -> Result<(), PluginError> {
        // 从配置文件读取插件条目（短锁：读取后立即释放，后续安装须重新加锁）。
        let plugin_entries = match store.list_plugins() {
            Ok(plugin_entries) => plugin_entries,
            Err(reason) => {
                log::error!(
                    "failed to read plugin entries from store during plugin startup: {reason}"
                );
                return Ok(());
            }
        };
        // 将插件装载进内存
        let mut plugins = HashMap::with_capacity(plugin_entries.len());
        for entry in &plugin_entries {
            let plugin_id = entry.id.clone();
            let plugin_dir = self.paths.plugin_dir(&plugin_id);
            let placed_plugin = match PlacedPlugin::from_plugin_dir(&plugin_dir) {
                Ok(placed_plugin) => placed_plugin,
                Err(err) => {
                    plugins.insert(plugin_id, PluginState::Error(err.to_string()));
                    continue;
                }
            };
            let plugin_state = match placed_plugin.load(&self.paths) {
                Ok(loaded_plugin) => {
                    if entry.enabled {
                        PluginState::Loaded(Arc::new(loaded_plugin))
                    } else {
                        PluginState::Disabled(loaded_plugin.meta)
                    }
                }
                Err(err) => PluginState::Error(err.to_string()),
            };
            plugins.insert(plugin_id, plugin_state);
        }

        if let Err(err) = self.entries.write().map(|mut entries| *entries = plugins) {
            log::error!("failed to lock plugin registry during startup: {err}");
            return Ok(());
        }

        // 筛选出配置中缺失的内置插件：随后的安装管线会为其落位并补写条目。
        let missing_builtins: Vec<String> = BUILTINS
            .iter()
            .filter(|plugin| !plugin_entries.iter().any(|entry| entry.id == plugin.id))
            .map(|plugin| plugin.source())
            .collect();

        for builtin_source in missing_builtins {
            if let Err(err) = self.add_plugin(store, &builtin_source) {
                log::warn!("failed to install builtin plugin {builtin_source}: {err}");
            }
        }
        Ok(())
    }

    /// 当前注册表视图。
    pub(crate) fn list(
        &self,
        store: &impl PluginEntries,
    ) -> Result<Vec<PluginRecord>, PluginError> {
        // 配置损坏时把原因原样带出去（哪个文件坏了、坏在哪）——「配置文件损坏」
        // 这四个字会让用户去猜是哪个文件。
        let plugin_entries = store.list_plugins().map_err(PluginError::from)?;
        let plugins = self
            .entries
            .read()
            .map_err(|_| PluginError::RegistryUnavailable)?;
        let plugin_record =
            |entry: &PluginEntry, meta: Option<&PluginMeta>, error: Option<String>| PluginRecord {
                source: entry.source.clone(),
                builtin: is_builtin_source(&entry.source),
                enabled: entry.enabled,
                id: entry.id.clone(),
                config_dir: meta.map(|m| m.config_dir.clone()),
                settings_schema: meta.and_then(|m| m.manifest.settings_schema.clone()),
                error,
            };
        Ok(plugin_entries
            .iter()
            .map(|entry| match plugins.get(&entry.id) {
                Some(PluginState::Loaded(loaded_plugin)) => {
                    plugin_record(entry, Some(&loaded_plugin.meta), None)
                }
                Some(PluginState::Disabled(meta)) => plugin_record(entry, Some(meta), None),
                Some(PluginState::Error(err)) => plugin_record(entry, None, Some(err.to_owned())),
                None => plugin_record(entry, None, Some(format!("插件 {} 未加载", entry.id))),
            })
            .collect())
    }
}
