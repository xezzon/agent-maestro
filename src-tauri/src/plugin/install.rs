//! 插件的安装与生命周期：获取 manifest 与 wasm、校验并落位、写配置条目，
//! 以及启用/禁用、移除与重新加载。
//!
//! 安装类流程共用 `install_lock` 独占临界区；配置存储只在短读/短写处加锁，
//! 获取、校验与落位全程不持锁。

use std::{fs, path::Path, sync::Arc};

use crate::{
    paths::MaestroPaths,
    store::{AppStore, PluginEntry, StoreError},
};

use super::{
    PlacedPlugin, PluginService, PluginState, builtin,
    manifest::{SourceKind, is_https_url, parse_manifest},
    remove_placed_dir,
};

impl PluginService {
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
    pub(crate) fn add_plugin(&self, store: &AppStore, source: &str) -> Result<(), String> {
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
        //    `load` 顺带解析 `config_dir`，解析先于 validate() 与 save()。
        let loaded = self.fetch_placed(source)?.load()?;
        // id 冲突预检：与写入时的复检问同一句规则（见 `Store::check_plugin_id_conflict`）。
        store
            .read()?
            .check_plugin_id_conflict(source, &loaded.manifest.id)?;
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
        log::debug!("fetching plugin artifacts: source={source}");

        // 内置来源按 source 精确查表分派（issue #56）：未知的 builtin:<id> 报错而非回落
        // 到某个内置插件——回落会把别人的 manifest 与 wasm 落位到该来源名下。
        let builtin = if source_kind == SourceKind::Builtin {
            Some(builtin::find(source).ok_or_else(|| format!("未知的内置插件来源：{source}"))?)
        } else {
            None
        };

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
            SourceKind::Builtin => builtin
                .expect("内置来源已在上面查表分派")
                .manifest_json
                .to_string(),
        };

        let manifest = parse_manifest(source_kind, &raw_manifest)?;

        let wasm = if let Some(builtin) = builtin {
            Vec::from(builtin.wasm)
        } else if is_https_url(&manifest.entry) {
            self.fetcher.fetch(&manifest.entry)?
        } else {
            // `source` 是 manifest.json 的路径，entry 相对其所在目录解析。
            let source_dir = Path::new(source).parent().unwrap_or(Path::new("."));
            let entry_path = source_dir.join(&manifest.entry);
            fs::read(&entry_path).map_err(|e| format!("读取 {} 失败：{e}", entry_path.display()))?
        };

        Ok(PlacedPlugin::new(
            &MaestroPaths::get().plugin_dir(&manifest.id),
            raw_manifest,
            &wasm,
        ))
    }

    /// 启用插件条目。
    ///
    /// 先从落位目录重新装载并解析 config_dir，装载成功才写配置条目并同步注册表
    /// ——装载失败即报错且不改配置，修复落位文件后重新启用即可。
    ///
    /// 整个流程在 `install_lock` 内独占：与添加、移除、重新加载互斥。
    /// 否则移除可能插进「写配置条目」与「同步注册表」之间，留下条目与落位文件
    /// 都已不在、注册表却仍记着已装载的僵尸项；重新加载则可能读走过期的
    /// `enabled` 并按它覆盖注册表，与刚落盘的配置对不上。
    /// 锁序 `install_lock` → store → entries，与其余安装类流程一致；
    /// 本流程只读本机文件、不联网，临界区比含下载的流程短。
    pub(crate) fn enable(&self, store: &AppStore, source: &str) -> Result<(), String> {
        let _install_guard = self.install_lock.lock().map_err(|_| "插件安装锁不可用")?;

        let entry = store.read()?.plugin_by_source(source)?;
        if entry.enabled {
            return Ok(());
        }

        // 启用先装载再写配置：装载失败不写条目、不动注册表，旧状态保持可用。
        let placed = PlacedPlugin::from_plugin_dir(&MaestroPaths::get().plugin_dir(&entry.id))?;
        let loaded = placed.load()?;

        let entry = {
            let mut guard = store.write()?;
            // 复检：条目可能已被移除，不得按过期条目继续写。
            // install_lock 已让移除与本流程互斥，此处是纵深防御。
            let entry = guard.plugin_by_source(source)?;
            if entry.enabled {
                // 并发方已完成同样的切换：配置与注册表均已同步，直接成功。
                return Ok(());
            }
            guard.set_plugin_enabled(source, true)?;
            entry
        };

        self.entries
            .write()
            .map_err(|_| "插件注册表不可用")?
            .insert(entry.id.clone(), PluginState::Loaded(Arc::new(loaded)));
        Ok(())
    }

    /// 禁用插件条目。
    ///
    /// 先改配置条目，再把内存注册表换成 Disabled 态（保留 manifest 供列表展示）。
    ///
    /// 整个流程在 `install_lock` 内独占：与添加、启用、移除、重新加载互斥
    /// （理由见 [`Self::enable`]）；本流程只读本机文件、不联网。
    pub(crate) fn disable(&self, store: &AppStore, source: &str) -> Result<(), String> {
        let _install_guard = self.install_lock.lock().map_err(|_| "插件安装锁不可用")?;

        let entry = store.read()?.plugin_by_source(source)?;
        if !entry.enabled {
            return Ok(());
        }

        let entry = {
            let mut guard = store.write()?;
            // 复检：条目可能已被移除，不得按过期条目继续写。
            let entry = guard.plugin_by_source(source)?;
            if !entry.enabled {
                // 并发方已完成同样的切换：配置与注册表均已同步，直接成功。
                return Ok(());
            }
            guard.set_plugin_enabled(source, false)?;
            entry
        };

        let mut entries = self.entries.write().map_err(|_| "插件注册表不可用")?;
        if let Some(state) = entries.get_mut(&entry.id)
            && let PluginState::Loaded(loaded) = state
        {
            let manifest = loaded.manifest.clone();
            *state = PluginState::Disabled(manifest);
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
        remove_placed_dir(&MaestroPaths::get().plugin_dir(&entry.id))
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
        let _install_guard = self.install_lock.lock().map_err(|_| "插件安装锁不可用")?;

        // 条目是来源到落位目录的唯一映射：未知来源无从重新装载。
        let entry = store.read()?.plugin_by_source(source)?;

        // 重新获取 manifest 与 wasm、校验：全程不持配置存储锁（与添加同构）。
        //    `load` 顺带解析 `config_dir`，解析先于 validate() 与 save()。
        let loaded = self.fetch_placed(source)?.load()?;
        // 上游 id 变更报错并保持旧状态：落位目录由条目 id 定位，id 漂移会架空映射。
        if loaded.manifest.id != entry.id {
            return Err(format!(
                "上游 manifest 的插件 id 已从「{}」变更为「{}」，拒绝重新加载",
                entry.id, loaded.manifest.id
            ));
        }
        // 链接期校验先于落位：wasm 不是组件或接口不兼容时不落位、注册表保持旧状态。
        // 不触发组件 init、不预开放真实配置目录，避免未授权写入。
        loaded.validate(&self.engine)?;
        // 逐文件覆盖落位产物：不触碰目录中宿主产物以外的文件（见 ADR 0018）。
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
}
