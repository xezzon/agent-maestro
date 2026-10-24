//! 插件的安装与生命周期：获取 manifest 与 wasm、校验并落位、写配置条目，
//! 以及启用/禁用、移除与重新加载。
//!
//! 安装类流程共用 `install_lock` 独占临界区；配置存储只在短读/短写处加锁，
//! 获取、校验与落位全程不持锁。

use std::{fs, path::Path, sync::Arc};

use crate::{
    errors::user_message,
    store::{AppStore, PluginEntry, StoreError},
};

use super::{
    INSTALL_LOCK_POISONED, LoadedPlugin, PlacedPlugin, PluginService, PluginState,
    REGISTRY_LOCK_POISONED, builtin,
    execution::validate_component,
    manifest::{Manifest, parse_manifest},
    remove_placed_dir,
    source::{PluginSource, is_https_url},
};

impl PluginService {
    /// 添加插件（内置、https 与本机路径来源同一流程）：
    /// 1. 先检查来源是否重复。
    /// 2. 随后获取 manifest 与 wasm、id 冲突检查、链接期校验，全部通过才落位。
    /// 3. 安装成功后写入配置条目；此前任何一步失败都不写条目、不留落位残留。
    ///
    /// 链接期校验（[`execution::validate_component`]）只做字节反序列化与导入/导出类型匹配，
    /// 不触发组件 init、不预开放真实配置目录。实投影阶段才完整实例化（见
    /// `write_providers`）。
    ///
    /// 整个流程在 `install_lock` 内独占：避免两个并发 install 各自落位后又被对方
    /// 的清理逻辑误删赢家的产物。配置存储只在短读/短写处加锁：获取、校验与落位
    /// 全程不持锁。
    pub(crate) fn add_plugin(&self, store: &AppStore, source: &str) -> Result<(), String> {
        let _install_guard = self
            .install_lock
            .lock()
            .map_err(|_| INSTALL_LOCK_POISONED)?;

        // 来源形态在发起获取之前判定：无法识别的来源直接拒绝。判定一次即成
        // 为值（`PluginSource`），后续流程只 match，不再重新解析同一串文本。
        let parsed = PluginSource::parse_or_err(source)?;
        let source_text = parsed.source();

        // 1. 来源即条目唯一身份：重复添加在发起下载之前拒绝。
        let duplicate = store
            .with_read(|s| s.list_plugins())
            .map_err(user_message)?
            .iter()
            .any(|plugin| plugin.source == source_text);
        if duplicate {
            return Err(user_message(StoreError::DuplicateSource {
                source: source_text,
            }));
        }

        // 2. 获取 manifest 与 wasm、校验并落位；获取与校验全程不持配置存储锁。
        //    manifest 只解析这一次：解析结果直接交给后面的校验与装配。
        let (placed, manifest) = self.fetch_placed(&parsed)?;
        // id 冲突预检：与写入时的复检问同一句规则（见 `Store::check_plugin_id_conflict`）。
        store
            .with_read(|s| s.check_plugin_id_conflict(&source_text, &manifest.id))
            .map_err(user_message)?;
        // 链接期校验先于解析配置目录与落位：wasm 不是组件或接口不兼容时不建
        // 目录、不落位、不写条目。仅做字节反序列化与类型检查，不触发组件 init、
        // 不预开放真实配置目录；避免未授权的 init 代码在安装失败时仍写入
        // plugin config_dir。
        validate_component(&self.engine, &placed.wasm, &manifest.id)?;
        // 解析配置目录：显式的一步（建目录在此发生），在链接期校验之后、落位之前。
        let config_dir = manifest.config_dir.resolve(&self.paths)?;
        let loaded = LoadedPlugin::from_parts(manifest, config_dir, placed);
        loaded.plugin.save()?;

        // 3. 安装成功后写入条目：`id` 是来源到落位目录的唯一映射（见 ADR 0006）。
        //    写入临界区内复检 id 冲突：预检与写入之间，另一来源可能已占用同一 id。
        let entry = PluginEntry {
            source: source_text,
            enabled: true,
            id: loaded.id().to_owned(),
        };
        if let Err(e) = store.with_write(|s| s.add_plugin(&entry)) {
            // 条目写不进去，就不能留下注册表看不见的落位目录。
            // 清理失败时把残留路径一并回报：避免注释承诺与实际行为偏离。
            if let Err(uninstall_err) = loaded.plugin.uninstall() {
                return Err(format!(
                    "{}；落位目录清理失败：{}（残留路径：{}）",
                    user_message(e.clone()),
                    uninstall_err,
                    loaded.plugin.path().display()
                ));
            }
            return Err(user_message(e));
        }
        self.entries
            .write()
            .map_err(|_| REGISTRY_LOCK_POISONED)?
            .insert(
                loaded.id().to_owned(),
                PluginState::Loaded(Arc::new(loaded)),
            );
        Ok(())
    }

    /// 从来源处获取 manifest 与 wasm，构造待落位的插件。
    ///
    /// 名为「获取」而非「下载」：https 来源走网络，file 来源读本机文件，
    /// 内置来源直接取内嵌字节（见 ADR 0004）。
    ///
    /// 来源已由调用方解析为 [`PluginSource`]：这里只按值分派，不再重新判。
    ///
    /// 一并返回**已解析的 manifest**：安装路径上 manifest 只解析这一次，解析
    /// 结果直接交给后面的校验与装配（不再由落位产物重新解析一遍）。
    fn fetch_placed(&self, source: &PluginSource) -> Result<(PlacedPlugin, Manifest), String> {
        let source_text = source.source();
        log::debug!("fetching plugin artifacts: source={source_text}");

        // 内置来源按 id 精确查表分派（issue #56）：未知的 builtin:<id> 报错而非回落
        // 到某个内置插件——回落会把别人的 manifest 与 wasm 落位到该来源名下。
        let (raw_manifest, builtin) = match source {
            PluginSource::Builtin(id) => {
                let builtin = builtin::find_by_id(id)
                    .ok_or_else(|| format!("未知的内置插件来源：{source_text}"))?;
                (builtin.manifest_json.to_string(), Some(builtin))
            }
            PluginSource::Https(url) => {
                let bytes = self
                    .fetcher
                    .fetch(url)
                    .map_err(|e| format!("下载 manifest 失败：{e}"))?;
                let raw_manifest = String::from_utf8(bytes)
                    .map_err(|e| format!("manifest.json 不是合法 UTF-8：{e}"))?;
                (raw_manifest, None)
            }
            PluginSource::File(path) => {
                let raw_manifest = fs::read_to_string(path)
                    .map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
                (raw_manifest, None)
            }
        };

        let manifest = parse_manifest(source, &raw_manifest)?;

        let wasm = match source {
            // 内置来源：wasm 内嵌于二进制，entry 不参与解析。
            PluginSource::Builtin(_) => match builtin {
                Some(builtin) => Vec::from(builtin.wasm),
                // 走到这里说明内置表里没有该 id（上面查表时已报错，此处只是让函数完备）。
                None => return Err(format!("未知的内置插件来源：{source_text}")),
            },
            PluginSource::Https(_) => self.fetcher.fetch(&manifest.entry)?,
            PluginSource::File(path) => {
                if is_https_url(&manifest.entry) {
                    self.fetcher.fetch(&manifest.entry)?
                } else {
                    // `source` 是 manifest.json 的路径，entry 相对其所在目录解析。
                    let source_dir = path.parent().unwrap_or(Path::new("."));
                    let entry_path = source_dir.join(&manifest.entry);
                    fs::read(&entry_path)
                        .map_err(|e| format!("读取 {} 失败：{e}", entry_path.display()))?
                }
            }
        };

        Ok((
            PlacedPlugin::new(&self.paths.plugin_dir(&manifest.id), raw_manifest, &wasm),
            manifest,
        ))
    }

    /// 启用插件条目：与禁用同一条流程（见 [`Self::set_enabled`]），只是方向不同。
    ///
    /// 先从落位目录重新装载并解析 config_dir，装载成功才写配置条目并同步注册表
    /// ——装载失败即报错且不改配置，修复落位文件后重新启用即可。
    pub(crate) fn enable(&self, store: &AppStore, source: &str) -> Result<(), String> {
        self.set_enabled(store, source, true)
    }

    /// 禁用插件条目：与启用同一条流程（见 [`Self::set_enabled`]），只是方向不同。
    ///
    /// 先改配置条目，再把内存注册表换成 Disabled 态（保留描述供列表展示）。
    pub(crate) fn disable(&self, store: &AppStore, source: &str) -> Result<(), String> {
        self.set_enabled(store, source, false)
    }

    /// 启用/禁用插件条目：两条流程的共同实现。
    ///
    /// 两条流程此前是两个逐段同构的方法（取锁 → 读条目 → 已处于目标态即返回
    /// → `with_write` 复检并写条目 → 更新注册表），差异只有「装载 vs 降级」与
    /// 方向。锁语义改一次要改两遍，且两处必须保持对称——现合成一处，差异
    /// 只在两处分叉：启用要先装载，禁用只降级注册表。
    ///
    /// 启用：从落位目录重新装载并解析 config_dir，装载成功才写配置条目并同步
    /// 注册表——装载失败即报错且不改配置，修复落位文件后重新启用即可。
    /// 禁用：先改配置条目，再把内存注册表换成 Disabled 态（保留描述供列表展示）。
    ///
    /// 整个流程在 `install_lock` 内独占：与添加、移除、重新加载互斥。
    /// 否则移除可能插进「写配置条目」与「同步注册表」之间，留下条目与落位文件
    /// 都已不在、注册表却仍记着已装载的僵尸项；重新加载则可能读走过期的
    /// `enabled` 并按它覆盖注册表，与刚落盘的配置对不上。
    /// 锁序 `install_lock` → store → entries，与其余安装类流程一致；
    /// 本流程只读本机文件、不联网，临界区比含下载的流程短。
    fn set_enabled(&self, store: &AppStore, source: &str, enabled: bool) -> Result<(), String> {
        let _install_guard = self
            .install_lock
            .lock()
            .map_err(|_| INSTALL_LOCK_POISONED)?;

        let entry = store
            .with_read(|s| s.plugin_by_source(source))
            .map_err(user_message)?;
        if entry.enabled == enabled {
            return Ok(());
        }

        // 启用先装载再写配置：装载失败不写条目、不动注册表，旧状态保持可用。
        // 禁用不装载：注册表里已有一份描述，降级即可。
        let loaded = if enabled {
            Some(
                PlacedPlugin::from_plugin_dir(&self.paths.plugin_dir(&entry.id))?
                    .load(&self.paths)?,
            )
        } else {
            None
        };

        // 复检：条目可能已被移除，不得按过期条目继续写。install_lock 已让
        // 移除与本流程互斥，此处是纵深防御。并发方已完成同样的切换时返回
        // `None`（配置与注册表均已同步，直接成功）。
        let entry = store
            .with_write(|guard| -> Result<Option<PluginEntry>, StoreError> {
                let entry = guard.plugin_by_source(source)?;
                if entry.enabled == enabled {
                    return Ok(None);
                }
                guard.set_plugin_enabled(source, enabled)?;
                Ok(Some(entry))
            })
            .map_err(user_message)?;
        let Some(entry) = entry else {
            return Ok(());
        };

        match loaded {
            Some(loaded) => {
                self.entries
                    .write()
                    .map_err(|_| REGISTRY_LOCK_POISONED)?
                    .insert(entry.id.clone(), PluginState::Loaded(Arc::new(loaded)));
            }
            None => {
                let mut entries = self.entries.write().map_err(|_| REGISTRY_LOCK_POISONED)?;
                if let Some(state) = entries.get_mut(&entry.id)
                    && let PluginState::Loaded(loaded) = state
                {
                    let meta = loaded.meta.clone();
                    *state = PluginState::Disabled(meta);
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
    pub(crate) fn remove_plugin(&self, store: &AppStore, source: &str) -> Result<(), String> {
        let _install_guard = self
            .install_lock
            .lock()
            .map_err(|_| INSTALL_LOCK_POISONED)?;

        // 内置插件随应用分发，不可移除（见 ADR 0004）。
        if matches!(PluginSource::parse(source), Some(PluginSource::Builtin(_))) {
            return Err(format!("内置插件不可移除：{source}"));
        }

        // 读条目、卸载注册表与删条目同处一个临界区（锁内读-改-写）；删落位
        // 目录是 IO，**不**在锁内——锁纪律由 `with_write` 的作用域保证。
        let entry = store.with_write(|guard| -> Result<Option<PluginEntry>, String> {
            let entry = match guard.plugin_by_source(source) {
                Ok(entry) => entry,
                // 条目已不存在：上一次移除已连同落位目录一并删除，幂等成功。
                Err(StoreError::MissingSource { .. }) => return Ok(None),
                Err(e) => return Err(user_message(e)),
            };

            // 1. 先从内存注册表卸载：此后删除期间插件不再可执行。
            self.entries
                .write()
                .map_err(|_| REGISTRY_LOCK_POISONED.to_owned())?
                .remove(&entry.id);

            // 2. 删配置条目（锁内读-改-写，与读条目同处一个临界区）。
            guard.delete_plugin(source).map_err(user_message)?;
            Ok(Some(entry))
        })?;
        let Some(entry) = entry else {
            return Ok(());
        };

        // 3. 删落位目录：只删宿主副本，不动用户的插件项目目录。
        remove_placed_dir(&self.paths.plugin_dir(&entry.id))
    }

    /// 重新加载：「按配置中的来源」无条件重新获取 manifest 与 wasm，按「先
    /// manifest、后 wasm」逐文件覆盖落位产物（见 ADR 0018）。写失败可能留下
    /// 「新 manifest + 旧 wasm」的混合态，恢复手段是从来源重新加载。
    /// 每次只作用于一个来源（见 ADR 0006）。
    ///
    /// 内置来源的“重新获取”即从内嵌字节重新构造落位副本，用于恢复被改动或
    /// 损坏的宿主副本（无上游新版本可言，但 ID 一致性校验仍执行）。
    ///
    /// 整个流程在 `install_lock` 内独占：与 add / remove 共用同一把锁，避免
    /// reload 写到一半时另一 add 把同一 id 的产物覆盖或被 remove 误删。
    pub(crate) fn reload_plugin(&self, store: &AppStore, source: &str) -> Result<(), String> {
        let _install_guard = self
            .install_lock
            .lock()
            .map_err(|_| INSTALL_LOCK_POISONED)?;

        // 条目是来源到落位目录的唯一映射：未知来源无从重新装载。
        let entry = store
            .with_read(|s| s.plugin_by_source(source))
            .map_err(user_message)?;

        // 重新获取 manifest 与 wasm、校验：全程不持配置存储锁（与添加同构）。
        //    manifest 只解析这一次，解析结果直接交给后面的校验与装配。
        let parsed = PluginSource::parse_or_err(source)?;
        let (placed, manifest) = self.fetch_placed(&parsed)?;
        // 上游 id 变更报错并保持旧状态：落位目录由条目 id 定位，id 漂移会架空映射。
        if manifest.id != entry.id {
            return Err(format!(
                "上游 manifest 的插件 id 已从「{}」变更为「{}」，拒绝重新加载",
                entry.id, manifest.id
            ));
        }
        // 链接期校验先于解析配置目录与落位：wasm 不是组件或接口不兼容时不建
        // 目录、不落位、注册表保持旧状态。不触发组件 init、不预开放真实配置
        // 目录，避免未授权写入。
        validate_component(&self.engine, &placed.wasm, &manifest.id)?;
        // 解析配置目录：显式的一步（建目录在此发生），在链接期校验之后、落位之前。
        let config_dir = manifest.config_dir.resolve(&self.paths)?;
        let loaded = LoadedPlugin::from_parts(manifest, config_dir, placed);
        // 逐文件覆盖落位产物：不触碰目录中宿主产物以外的文件（见 ADR 0018）。
        loaded.plugin.save()?;

        // 条目（id 与 enabled）不变，仅把内存注册表同步为新装载的版本。
        let state = if entry.enabled {
            PluginState::Loaded(Arc::new(loaded))
        } else {
            PluginState::Disabled(loaded.meta)
        };
        self.entries
            .write()
            .map_err(|_| REGISTRY_LOCK_POISONED)?
            .insert(entry.id, state);
        Ok(())
    }
}
