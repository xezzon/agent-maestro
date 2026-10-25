//! 配置存储：启动时从磁盘加载进内存，变更后原子写回。
//!
//! 模块按职责拆开：
//! - [`protected`]：保护态（配置文件损坏时拒绝一切读写）；
//! - [`persist`]：原子落盘（临时文件 + rename，写法见 crate 级 `fs_util`）；
//! - [`providers`]：Provider 仓储（新增/更新/删除/启停 + 落盘前的一致性校验）；
//! - [`plugins`]：插件条目仓储（新增/删除/启停）；
//! - [`variable_table`]：变量表（整包替换 + 名合法性）。
//!
//! 配置文档的**形状**（顶层文档、插件条目）住在 crate 根的
//! [`crate::config`]：存储层与插件服务两侧都消费它，故它不属于任何一侧；
//! 错误类型住在 [`crate::errors`]，同样两侧共享。

mod persist;
mod plugins;
mod ports;
mod protected;
mod providers;
mod variable_table;

#[cfg(test)]
mod tests;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::RwLock,
};

use crate::{
    config::{CONFIG_VERSION, Change, Config, PluginEntry},
    errors::{ConfigError, LockPoisoned, PluginEntryError, ProviderError, VariableError},
    provider::Provider,
    variables::Variables,
};

use protected::ProtectedState;

/// 配置存储：启动时从磁盘加载进内存，变更后原子写回。
///
/// 配置文件损坏（无法解析或版本不受支持）时进入保护状态：
/// 读取与写入一律报错，绝不静默重建或覆盖原文件。
pub(crate) struct Store {
    config_path: PathBuf,
    state: ProtectedState,
}

impl Store {
    /// 从给定路径加载配置。文件不存在视为首次使用（空配置）；
    /// 损坏则进入保护状态。
    ///
    /// 路径由构造方（`lib.rs` 的装配、测试夹具）显式传入：构造函数不再自己去
    /// 取全局环境，依赖在签名上可见，测试也能指向任意配置文件（#141 I-4）。
    pub(crate) fn new(config_path: PathBuf) -> Self {
        let state = match fs::read_to_string(&config_path) {
            Ok(text) => Self::parse(&config_path, &text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(ConfigError::io(&config_path, e.to_string())),
        };
        Self {
            config_path,
            state: match state {
                Ok(config) => ProtectedState::Ready(config),
                Err(reason) => ProtectedState::unavailable(reason),
            },
        }
    }

    fn parse(path: &Path, text: &str) -> Result<Config, ConfigError> {
        let config: Config =
            serde_json::from_str(text).map_err(|e| ConfigError::corrupt(path, e.to_string()))?;
        if config.version != CONFIG_VERSION {
            return Err(ConfigError::UnsupportedVersion {
                path: path.to_owned(),
                found: config.version,
            });
        }
        Ok(config)
    }

    /// 当前配置；存储处于保护状态时返回错误。
    ///
    /// 错误按值返回：调用方拿到的是一份自有值，交给命令层即可，不必为
    /// 「把借用交出去」而四处 `map_err(Clone::clone)`。本方法**模块私有**——
    /// 外部只经由语义化查询（`providers_snapshot` / `enabled_providers` /
    /// `variables_snapshot` / `list_plugins` / `config_snapshot`）读取配置，
    /// 不直接触碰 `Config` 的字段布局。
    fn get(&self) -> Result<&Config, ConfigError> {
        self.state.config()
    }

    /// 整份配置快照（自有值）：给「要整份配置」的调用方（启动摘要、测试夹具）。
    ///
    /// 只需要其中一段的调用方走语义化查询，不必克隆整份配置。
    pub(crate) fn config_snapshot(&self) -> Result<Config, ConfigError> {
        Ok(self.get()?.clone())
    }

    /// Provider 表快照（按 slug 键序）。
    pub(crate) fn providers_snapshot(&self) -> Result<BTreeMap<String, Provider>, ConfigError> {
        Ok(self.get()?.providers.clone())
    }

    /// 参与投影的 Provider 表快照（#94）：`enabled == false` 者整体不参与——
    /// 既不写入插件，也不做占位符插值，因此其字段里的占位符错误不会牵连整个
    /// 插件失败。筛选与快照合并为一次克隆，且只克隆启用项。
    pub(crate) fn enabled_providers(&self) -> Result<BTreeMap<String, Provider>, ConfigError> {
        Ok(providers::select_enabled_providers(&self.get()?.providers))
    }

    /// 全局变量表快照（ADR 0015）。
    pub(crate) fn variables_snapshot(&self) -> Result<Variables, ConfigError> {
        Ok(self.get()?.variables.clone())
    }

    /// 唯一的配置写路径：取当前配置 → clone → 交仓储改 → 落盘 → 换状态。
    ///
    /// 闭包只写自己的领域变更与校验，「要不要落盘」由返回值决定
    /// （`Change::Unchanged` 即无变化：不写盘、不换状态）。
    /// 任何失败——保护态、校验不通过、目标不存在、落盘 IO 错误——都不改状态：
    /// 草稿随之丢弃，状态保持原样。
    fn mutate<T, E>(&mut self, f: impl FnOnce(&mut Config) -> Result<Change<T>, E>) -> Result<T, E>
    where
        E: From<ConfigError>,
    {
        let mut next = self.get().map_err(E::from)?.clone();
        match f(&mut next)? {
            Change::Unchanged(value) => Ok(value),
            Change::Changed(value) => {
                self.persist(&next).map_err(E::from)?;
                self.state = ProtectedState::Ready(next);
                Ok(value)
            }
        }
    }

    /// 原子写入：先写同目录临时文件并落盘，再 rename 覆盖目标，避免半截文件。
    fn persist(&self, config: &Config) -> Result<(), ConfigError> {
        persist::persist(&self.config_path, config)
    }

    /// 新建一条 Provider（slug 唯一），成功后原子写回磁盘。
    /// 落盘前对当前变量表插值一次：占位符引用必须可解析（变量已定义，或带
    /// 内联默认值），语法错误同样拦截——provider↔variables 一致性；
    /// 禁用状态不影响该校验。
    pub(crate) fn create_provider(
        &mut self,
        slug: &str,
        provider: Provider,
    ) -> Result<(), ProviderError> {
        self.mutate(|config| providers::create(config, slug, provider))
    }

    /// 整包替换指定 Provider 的端点、模型列表与 API Key（明文）。
    /// slug 不存在时报错，不做 upsert；落盘前对当前变量表插值一次，占位符
    /// 引用必须可解析（同 `create_provider`，禁用状态不影响校验）。
    /// 返回被替换的原记录。
    pub(crate) fn update_provider(
        &mut self,
        slug: &str,
        provider: Provider,
    ) -> Result<Provider, ProviderError> {
        self.mutate(|config| providers::update(config, slug, provider))
    }

    /// 删除 Provider，其端点、模型与 API Key 随记录一并移除，不留孤儿数据。
    /// 返回被删除的记录。
    pub(crate) fn delete_provider(&mut self, slug: &str) -> Result<Provider, ProviderError> {
        self.mutate(|config| providers::delete(config, slug))
    }

    /// 启用/禁用单个 Provider（独立于编辑表单的开关）。
    ///
    /// 幂等：状态未变即直接成功、不写盘。切换不改变占位符文本，而存储写出的每条
    /// 记录都已通过落盘一致性校验（create/update/set_variables 均不豁免禁用项），
    /// 故切换无需再次校验插值。
    pub(crate) fn set_provider_enabled(
        &mut self,
        slug: &str,
        enabled: bool,
    ) -> Result<(), ProviderError> {
        self.mutate(|config| providers::set_enabled(config, slug, enabled))
    }

    /// 整包替换全局变量表（ADR 0015）：新增、修改与删除都走同一条全量保存路径。
    /// 变量名须合法（`[A-Za-z0-9_]+`，ADR 0014），否则占位符无法引用；落盘前
    /// 用新表对全部存量 Provider（含禁用者）插值一次：删除导致任一 Provider 的
    /// 引用无法解析时整表拒绝（provider↔variables 一致性）。空默认值合法
    /// （`${EMPTY}`）。
    pub(crate) fn set_variables(&mut self, variables: Variables) -> Result<(), VariableError> {
        // 名校验必须先于存储状态：`mutate` 第一句是 `get()?`，保护态会抢先报
        // `Corrupt`。非法名整表拒绝，与当前配置是否可读无关（见
        // `variable_table::validate_names`）。
        variable_table::validate_names(&variables)?;
        self.mutate(|config| variable_table::replace(config, variables))
    }

    /// 新增插件条目，追加在现有条目之后。
    ///
    /// 安装成功后由插件服务调用，`id` 随条目一并给出：它是来源到落位目录的唯一映射
    /// （见 ADR 0006）。因此安装失败时不会留下条目，重新添加即可。
    ///
    /// 同一临界区内复检来源与 id 冲突：插件服务的预检（`check_id_conflict`）与
    /// 此处写入分属两次加锁，两次锁之间另一来源可能已占用同一 id。
    pub(crate) fn add_plugin(
        &mut self,
        plugin_entry: &PluginEntry,
    ) -> Result<(), PluginEntryError> {
        self.mutate(|config| plugins::add(config, plugin_entry))
    }

    /// 删除插件条目，返回被删除的条目（调用方据此删除落位目录）。
    pub(crate) fn delete_plugin(&mut self, source: &str) -> Result<PluginEntry, PluginEntryError> {
        self.mutate(|config| plugins::delete(config, source))
    }

    pub(crate) fn list_plugins(&self) -> Result<Vec<PluginEntry>, ConfigError> {
        Ok(self.get()?.plugins.clone())
    }

    /// 按来源取插件条目（`source` 即条目唯一身份）。
    pub(crate) fn plugin_by_source(&self, source: &str) -> Result<PluginEntry, PluginEntryError> {
        self.get()?
            .plugins
            .iter()
            .find(|plugin| plugin.source == source)
            .cloned()
            .ok_or_else(|| PluginEntryError::missing_source(source))
    }

    /// 启用/禁用插件条目。
    pub(crate) fn set_plugin_enabled(
        &mut self,
        source: &str,
        enabled: bool,
    ) -> Result<(), PluginEntryError> {
        self.mutate(|config| plugins::set_enabled(config, source, enabled))
    }

    /// 插件 id 冲突预检：同一 id 不得被**其它**来源的条目持有。
    ///
    /// 安装流程在落位前问一次、写入时再问一次（两次加锁之间可能被抢占）——
    /// 两处问的是同一句规则，故错误类型也统一为 [`PluginEntryError::DuplicateId`]。
    pub(crate) fn check_plugin_id_conflict(
        &self,
        source: &str,
        id: &str,
    ) -> Result<(), PluginEntryError> {
        plugins::check_id_conflict(self.get()?, source, id)
    }
}

/// 共享应用状态：配置存储（启动时加载进内存，变更后原子写回）。
pub(crate) struct AppStore {
    /// 读多写少：list_plugins、plugin_by_source、provider 列表等只读路径在
    /// 同一 Store 上并发；写操作（add_plugin / set_plugin_enabled /
    /// create_provider 等）走 `AppStore::write()` 独占。
    pub(crate) store: RwLock<Store>,
}

impl AppStore {
    /// 在读锁内执行闭包：闭包的返回值即脱离锁的那份自有值。
    ///
    /// 锁守卫不出作用域——调用方拿不到守卫，自己的 IO 便不会落在锁内。
    /// 插件服务够不到这一层：它经 [`crate::plugin::ports`] 的端口访问配置
    /// 存储，锁与落盘时机都在适配器一侧（#141 I-1）。
    ///
    /// 锁边界不擦除错误类型（#141 I-3）：闭包的错误类型 `E` 原样透传——存储
    /// 错误（[`ConfigError`] / [`ProviderError`] / [`PluginEntryError`] /
    /// [`VariableError`]）由命令层经其 `Display` 归一为面向用户的文案；
    /// 锁毒化经 [`LockPoisoned`] 折进 `E`（各错误类型各一段 `From` 实现）。
    pub(crate) fn with_read<T, E>(&self, f: impl FnOnce(&Store) -> Result<T, E>) -> Result<T, E>
    where
        E: From<LockPoisoned>,
    {
        let guard = self.store.read().map_err(|_| LockPoisoned)?;
        f(&guard)
    }

    /// 在写锁内执行闭包：独占期间所有读守卫与其它写守卫均需等待。
    ///
    /// 锁被毒化同样报错（经 [`LockPoisoned`] 折进闭包的错误类型）。写路径的
    /// 原子落盘由 [`Store`] 在闭包返回后完成（见 [`Store::mutate`]），但那一步
    /// 仍在这把锁内——落盘期间持锁是既有取舍（写路径要「改内存 + 落盘」原子
    /// 完成）。锁守卫不出作用域，故调用方自己的 IO 不会落在锁内。
    pub(crate) fn with_write<T, E>(
        &self,
        f: impl FnOnce(&mut Store) -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<LockPoisoned>,
    {
        let mut guard = self.store.write().map_err(|_| LockPoisoned)?;
        f(&mut guard)
    }
}
