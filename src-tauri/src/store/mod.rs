//! 配置存储：启动时从磁盘加载进内存，变更后原子写回。
//!
//! 模块按职责拆开：
//! - [`protected`]：保护态（配置文件损坏时拒绝一切读写）；
//! - [`migrate`]：启动时把旧明文 API Key 迁进密钥链（尽力而为，失败保留）；
//! - [`persist`]：原子落盘（临时文件 + rename，写法见 crate 级 `fs_util`）；
//! - [`providers`]：Provider 的领域规则（参与投影的筛选、落盘前的一致性校验、
//!   启停）；
//! - [`secrets`]：Provider 凭证的写入（密钥链优先、配置只留引用、失败回滚）；
//! - [`plugins`]：插件条目仓储（新增/删除/启停）；
//! - [`variable_table`]：变量表（整包替换 + 名合法性）。
//!
//! 配置文档的**形状**（顶层文档、插件条目）住在 crate 根的
//! [`crate::config`]：存储层与插件服务两侧都消费它，故它不属于任何一侧；
//! 错误类型住在 [`crate::errors`]，同样两侧共享。

mod migrate;
mod persist;
mod plugins;
mod ports;
mod protected;
mod providers;
mod secrets;
mod variable_table;

#[cfg(test)]
pub(crate) mod testutil;

#[cfg(test)]
mod tests;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use crate::{
    config::{CONFIG_VERSION, Change, Config, MIN_CONFIG_VERSION, PluginEntry},
    errors::{ConfigError, LockPoisoned, PluginEntryError, ProviderError, VariableError},
    provider::{ApiKeyIntent, Provider, ProviderWrite},
    secret::{SecretStore, SecretText},
    variables::Variables,
};

use protected::ProtectedState;
use secrets::{Rollback, SecretWrite};

/// 配置存储：启动时从磁盘加载进内存，变更后原子写回。
///
/// 配置文件损坏（无法解析或版本不受支持）时进入保护状态：
/// 读取与写入一律报错，绝不静默重建或覆盖原文件。
///
/// 凭证的真值落在系统密钥链（[`SecretStore`]），配置里只留引用
/// （ADR 0002）：密钥链由构造方注入，生产实现是系统密钥链，测试实现是内存
/// 替身。
pub(crate) struct Store {
    config_path: PathBuf,
    state: ProtectedState,
    /// 系统密钥链：真值的唯一落点（ADR 0002）。
    secrets: Arc<dyn SecretStore>,
}

impl Store {
    /// 从给定路径加载配置。文件不存在视为首次使用（空配置）；
    /// 损坏则进入保护状态。
    ///
    /// 路径由构造方（`lib.rs` 的装配、测试夹具）显式传入：构造函数不再自己去
    /// 取全局环境，依赖在签名上可见，测试也能指向任意配置文件（#141 I-4）。
    ///
    /// 密钥链同样由构造方注入：生产装配传系统密钥链，测试夹具传内存替身。
    ///
    /// 构造的最后一步是**启动迁移**（#147）：配置里的旧明文 API Key 一次性
    /// 换进密钥链，配置文件随之只剩引用。迁移在任何失败下都不让启动失败
    /// ——密钥链不可用时配置里仍是明文，下次启动再试。
    pub(crate) fn new(config_path: PathBuf, secrets: Arc<dyn SecretStore>) -> Self {
        let state = match fs::read_to_string(&config_path) {
            Ok(text) => Self::parse(&config_path, &text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(ConfigError::io(&config_path, e.to_string())),
        };
        let mut store = Self {
            config_path,
            state: match state {
                Ok(config) => ProtectedState::Ready(config),
                Err(reason) => ProtectedState::unavailable(reason),
            },
            secrets,
        };
        store.migrate_plaintext_secrets();
        store
    }

    /// 启动时迁移旧明文 API Key（#147）。
    ///
    /// 逐 Provider 尽力而为：换成功的字段落成规范引用，换不成的保留明文
    /// 原样并记一条 ERROR（只含 slug 与原因，绝不含密钥值）。全部尝试完
    /// **只落盘一次**：成功的写引用、失败的写回明文。
    ///
    /// 落盘失败则内存状态保持迁移前的样子（配置压根没被换掉），而本次写进
    /// 密钥链的条目**不回收**——惰性残余，同 slug 下次写入时即被覆盖
    /// （ADR 0002 接受的风险）。
    ///
    /// 走 [`Store::mutate`] 这条唯一的配置写路径：它「落盘失败即不改状态」
    /// 的语义正是迁移要的回滚。
    fn migrate_plaintext_secrets(&mut self) {
        if self.state.config().is_err() {
            // 保护态：配置读不出来，没什么可迁移的。
            return;
        }
        let secrets = self.secrets.clone();
        let mut migrated_slugs: Vec<String> = Vec::new();
        let outcome = self.mutate::<(), ConfigError>(|config| {
            let migration = migrate::migrate(config, secrets.as_ref());
            for failure in &migration.failures {
                log::error!(
                    "secret migration failed: slug={}: {}",
                    failure.slug,
                    failure.reason
                );
            }
            migrated_slugs = migration.migrated_slugs;
            if migrated_slugs.is_empty() {
                // 一个都没迁成：配置一个字都没动，不必落盘。
                Ok(Change::Unchanged(()))
            } else {
                Ok(Change::Changed(()))
            }
        });
        if let Err(error) = outcome {
            // 落盘失败：内存状态仍是迁移前的样子（配置没被换掉），密钥链
            // 条目留着——下次启动重试时同 slug 覆盖它。
            //
            // 闭包不会失败，故此处只剩落盘这一种病因（保护态在上面就挡住了）。
            log::error!(
                "secret migration persist failed: slugs={}: {error}",
                migrated_slugs.join(",")
            );
        }
    }

    fn parse(path: &Path, text: &str) -> Result<Config, ConfigError> {
        let config: Config =
            serde_json::from_str(text).map_err(|e| ConfigError::corrupt(path, e.to_string()))?;
        if config.version < MIN_CONFIG_VERSION || config.version > CONFIG_VERSION {
            return Err(ConfigError::UnsupportedVersion {
                path: path.to_owned(),
                found: config.version,
                supported: MIN_CONFIG_VERSION..=CONFIG_VERSION,
            });
        }
        // 引用与它所在 map key 推出的 slug 必须一致：不一致即整份配置进保护态
        // （GLOSSARY「密钥引用」：引用与 slug 绑定）。
        for (slug, provider) in &config.providers {
            if let Some(reference) = provider.api_key.reference()
                && reference.slug() != slug
            {
                return Err(ConfigError::SecretReferenceMismatch {
                    path: path.to_owned(),
                    slug: slug.clone(),
                    reference_slug: reference.slug().to_owned(),
                });
            }
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
                self.persist(&mut next).map_err(E::from)?;
                self.state = ProtectedState::Ready(next);
                Ok(value)
            }
        }
    }

    /// 原子写入：先算出落盘版本（内容含引用即 v2），再写同目录临时文件并落盘，
    /// 最后 rename 覆盖目标，避免半截文件。
    ///
    /// 版本在落盘前定：配置里的版本号必须与内容相符（见
    /// [`crate::config::persist_version`]），否则旧版 Maestro 会把引用当成
    /// API Key 投影出去。
    fn persist(&self, config: &mut Config) -> Result<(), ConfigError> {
        config.version = crate::config::persist_version(config);
        persist::persist(&self.config_path, config)
    }

    /// 新建一条 Provider（slug 唯一），成功后原子写回磁盘。
    ///
    /// 凭证按三态契约落到密钥链（见 [`ApiKeyIntent`]）：缺省与清除都落成
    /// 未设置，覆盖则**先写密钥链、再落引用**。落盘前对当前变量表插值一次：
    /// 占位符引用必须可解析（变量已定义，或带内联默认值），语法错误同样
    /// 拦截——provider↔variables 一致性；禁用状态不影响该校验。
    pub(crate) fn create_provider(
        &mut self,
        slug: &str,
        write: ProviderWrite,
    ) -> Result<(), ProviderError> {
        let next = self.get().map_err(ProviderError::from)?.clone();
        if next.providers.contains_key(slug) {
            return Err(ProviderError::duplicate_slug(slug));
        }
        providers::validate_provider(slug, &write.provider, &next.variables)
            .map_err(ProviderError::Interpolation)?;
        self.commit_provider(slug, next, write, None)
    }

    /// 整包替换指定 Provider 的端点、模型列表与凭证。
    /// slug 不存在时报错，不做 upsert；落盘前对当前变量表插值一次，占位符
    /// 引用必须可解析（同 `create_provider`，禁用状态不影响校验）。
    /// 返回被替换的原记录。
    pub(crate) fn update_provider(
        &mut self,
        slug: &str,
        write: ProviderWrite,
    ) -> Result<Provider, ProviderError> {
        let next = self.get().map_err(ProviderError::from)?.clone();
        let original = next
            .providers
            .get(slug)
            .cloned()
            .ok_or_else(|| ProviderError::missing_slug(slug))?;
        providers::validate_provider(slug, &write.provider, &next.variables)
            .map_err(ProviderError::Interpolation)?;
        self.commit_provider(slug, next, write, Some(&original.api_key))?;
        Ok(original)
    }

    /// 删除 Provider，其端点、模型与凭证随记录一并移除，不留孤儿数据。
    ///
    /// 密钥链条目只在记录持有引用时才级联删除；明文与未设置的记录压根没有
    /// 条目可删，故不碰密钥链。返回被删除的记录。
    pub(crate) fn delete_provider(&mut self, slug: &str) -> Result<Provider, ProviderError> {
        let mut next = self.get().map_err(ProviderError::from)?.clone();
        let removed = next
            .providers
            .get(slug)
            .cloned()
            .ok_or_else(|| ProviderError::missing_slug(slug))?;
        // 先删条目、再落配置：密钥链不可用即失败，Provider 保留。
        let rollback = secrets::cascade(&removed.api_key, self.secrets.as_ref())
            .map_err(|error| ProviderError::secret(slug, error))?;
        next.providers.remove(slug);
        self.persist_credential(slug, &mut next, rollback)?;
        self.state = ProtectedState::Ready(next);
        Ok(removed)
    }

    /// 写一条 Provider 记录：凭证先落密钥链，配置里只留引用。
    ///
    /// `existing` 是记录里现有的字段值（新建即 `None`）：写入意图按它决定
    /// 落成什么——不变沿用旧值、清除删条目、覆盖写条目。
    ///
    /// 凭证先落密钥链：密钥链失败即整体失败，配置一动不动。落盘失败按旧值
    /// 回滚密钥链；回滚也失败则两次失败一并点名。
    fn commit_provider(
        &mut self,
        slug: &str,
        mut next: Config,
        write: ProviderWrite,
        existing: Option<&SecretText>,
    ) -> Result<(), ProviderError> {
        let SecretWrite { value, rollback } =
            self.write_credential(slug, &write.api_key, existing)?;
        let mut provider = write.provider;
        provider.api_key = value;
        next.providers.insert(slug.to_owned(), provider);
        self.persist_credential(slug, &mut next, rollback)?;
        self.state = ProtectedState::Ready(next);
        Ok(())
    }

    /// 凭证的写入：一次分派管住新建与更新。
    ///
    /// `existing` 是记录里现有的字段值（新建即 `None`）：
    ///
    /// - **不变**：沿用旧值；新建没有旧值，落成未设置；
    /// - **清除**：只有**引用**才指向密钥链条目，明文与未设置没有条目可删——
    ///   不碰密钥链。于是「保存一个没有凭证的 Provider」不依赖密钥链可用，
    ///   密钥链不可用时挂掉的只是「带密钥的保存」；
    /// - **覆盖**：先写密钥链，再落规范引用。
    fn write_credential(
        &self,
        slug: &str,
        intent: &ApiKeyIntent,
        existing: Option<&SecretText>,
    ) -> Result<SecretWrite, ProviderError> {
        match intent {
            ApiKeyIntent::Unchanged => Ok(match existing {
                Some(existing) => SecretWrite::keep(existing),
                None => SecretWrite::unset(),
            }),
            ApiKeyIntent::Clear => match existing.and_then(SecretText::reference) {
                Some(reference) => secrets::clear(reference, self.secrets.as_ref())
                    .map_err(|error| ProviderError::secret(slug, error)),
                None => Ok(SecretWrite::unset()),
            },
            ApiKeyIntent::Overwrite(value) => {
                let reference = secrets::canonical(slug)?;
                secrets::overwrite(&reference, value, self.secrets.as_ref())
                    .map_err(|error| ProviderError::secret(slug, error))
            }
        }
    }

    /// 落盘并处理密钥链回滚。
    ///
    /// 落盘失败 → 用旧值回滚密钥链（旧值不存在就删掉新值）；回滚本身也失败
    /// → 两次失败一并点名（配置没改动，密钥链却可能与配置不一致）。
    fn persist_credential(
        &self,
        slug: &str,
        config: &mut Config,
        rollback: Option<Rollback>,
    ) -> Result<(), ProviderError> {
        match self.persist(config) {
            Ok(()) => Ok(()),
            Err(persist_error) => match rollback {
                None => Err(ProviderError::Storage(persist_error)),
                Some(rollback) => match rollback.apply(self.secrets.as_ref()) {
                    Ok(()) => Err(ProviderError::Storage(persist_error)),
                    Err(rollback_error) => Err(ProviderError::SecretRollback {
                        slug: slug.to_owned(),
                        persist: Box::new(persist_error),
                        rollback: rollback_error,
                    }),
                },
            },
        }
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
    /// 密钥链句柄：**另持一份**，不随配置锁进出。
    ///
    /// 投影前解析密钥时用它取回真值（#146）。读取发生在配置锁之外——密钥链
    /// 后端的延迟因此不会占着配置的读锁，其它命令照常读写配置。
    secrets: Arc<dyn SecretStore>,
}

impl AppStore {
    /// 装配：配置存储与密钥链句柄。
    ///
    /// 两份密钥链句柄指向**同一个**后端：存储层用它写，投影路径用它读。
    pub(crate) fn new(store: Store, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            store: RwLock::new(store),
            secrets,
        }
    }

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
