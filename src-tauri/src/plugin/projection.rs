//! 投影：把 Provider 写入各插件的配置目录，并把插件回报的相对路径拼成宿主绝对路径。

use std::{
    collections::BTreeMap,
    path::{Component, Path},
    sync::Arc,
};

use crate::{
    interpolate,
    paths::MaestroPaths,
    provider::Provider,
    store::{AppStore, StoreError},
    variables::Variables,
};

use super::{
    ApplyStatus, LoadedPlugin, PluginApplyReport, PluginService, PluginState,
    REGISTRY_LOCK_POISONED, config::read_plugin_config, execution::SkippedProvider,
};

impl PluginService {
    /// 应用到工具：把 Provider 投影进各插件，返回逐插件结果。
    ///
    /// 禁用 Provider（`enabled == false`）在读取配置时即剔除（#94），不进入插值
    /// 与写入。store 锁在读取 providers 后即释放，注册表锁也仅用于快照：
    /// 插件执行时长不受应用控制，执行全程不持锁，不得阻塞其它命令。
    pub(crate) fn apply_providers(
        &self,
        store: &AppStore,
    ) -> Result<Vec<PluginApplyReport>, String> {
        let (providers, variables) = store.with_read(|s| -> Result<_, StoreError> {
            Ok((s.enabled_providers()?, s.variables_snapshot()?))
        })?;
        let reports = self.write_providers(&providers, &variables)?;
        // 逐插件的失败不是命令的失败（命令整体成功），故不进命令层的错误路径：
        // 在此按插件各记一条，失败原因与插件 id 一起留在日志里。
        for report in &reports {
            if report.status != ApplyStatus::Applied {
                log::warn!(
                    "plugin projection {status}: id={id} reason={reason}",
                    status = report.status,
                    id = report.id,
                    reason = report.reason.as_deref().unwrap_or(""),
                );
            }
        }
        Ok(reports)
    }

    /// 对注册表中的每个插件执行投影，返回逐插件报告：
    /// Loaded 交给 [`Self::project_one_plugin`]；Disabled 与 Error 态不执行投影。
    /// 单个插件的失败不影响其它插件，失败原因写入该插件的报告——即该插件整体
    /// `failed`、不写任何文件（判定细则见 [`Self::project_one_plugin`]）。
    ///
    /// 传入的每个 Provider 都参与投影：是否禁用由调用方先行剔除（#94 在命令层
    /// `enabled_providers` 完成），本函数不感知 `enabled`。
    ///
    /// 插件执行时长不受应用控制：注册表锁只在快照阶段短暂持有（克隆 Arc 与
    /// 收集跳过态），wasm 实例化与调用全部在锁外进行，不阻塞其它命令。
    pub(crate) fn write_providers(
        &self,
        providers: &BTreeMap<String, Provider>,
        variables: &Variables,
    ) -> Result<Vec<PluginApplyReport>, String> {
        let (mut reports, loaded) = {
            let entries = self.entries.read().map_err(|_| REGISTRY_LOCK_POISONED)?;
            let mut reports = Vec::with_capacity(entries.len());
            let mut loaded = Vec::new();
            for (id, state) in entries.iter() {
                match state {
                    PluginState::Loaded(loaded_plugin) => {
                        loaded.push((id.clone(), Arc::clone(loaded_plugin)));
                    }
                    PluginState::Disabled(_) => {
                        reports.push(skipped_report(id.clone(), "插件已禁用".to_owned()))
                    }
                    PluginState::Error(reason) => {
                        reports.push(skipped_report(id.clone(), reason.clone()))
                    }
                }
            }
            (reports, loaded)
        };

        for (id, loaded_plugin) in loaded {
            log::debug!("projecting providers: id={id}");
            let projected = self.project_one_plugin(&id, &loaded_plugin, providers, variables);
            reports.push(build_report(id, projected));
        }
        Ok(reports)
    }

    /// 对一个已加载插件完成完整投影：读插件配置 → 合并变量覆盖 → 序列化 form →
    /// 插值 → 实例化 → 写投影 → 拼绝对路径。成功返回已写入文件的宿主绝对路径与
    /// 被跳过的 Provider。
    ///
    /// 投影按插件读取插件配置（ADR 0018）：全局变量表 ⊕ 该插件的 `variables` 覆盖
    /// 后插值（同一占位符可按插件解析出不同值），`form` 段在组件实例化之前就位、
    /// 经 `get-config` import 递送；先插值、后映射端点（ADR 0015）、端点选择见
    /// ADR 0016。插件配置读取或解析失败、任一字段插值失败即该插件整体 `failed`
    /// ——不实例化、不调用插件、不写任何文件。
    fn project_one_plugin(
        &self,
        id: &str,
        loaded_plugin: &LoadedPlugin,
        providers: &BTreeMap<String, Provider>,
        variables: &Variables,
    ) -> Result<(Vec<String>, Vec<SkippedProvider>), String> {
        // 按插件读取插件配置（ADR 0018）：读取或解析失败即整插件失败，不静默降级。
        let plugin_config = read_plugin_config(&MaestroPaths::get().plugin_dir(id))?;
        // 全局变量表 ⊕ 该插件的变量覆盖：覆盖键优先，只影响本插件的插值。
        // config.json 容许残留全局表中已不存在的键，但不参与插值。
        let mut merged = variables.clone();
        merged.extend(
            plugin_config
                .variables
                .into_iter()
                .filter(|(name, _)| variables.contains_key(name)),
        );
        // form 段以原始 JSON 文本在实例化之前就位（宿主不解释其内容，ADR 0017）。
        let form = plugin_config
            .form
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| format!("序列化插件 form 失败：{e}"))?;
        let projected = interpolate::interpolate_providers(providers, &merged)?;
        let (files, skipped) = loaded_plugin
            .instantiate_component(&self.engine, form)?
            .write_provider(&projected)?;
        // 插件返回的是相对 config_dir 的路径：拼成宿主绝对路径回报给界面。
        let files = project_files(&loaded_plugin.manifest.config_dir, files)?;
        Ok((files, skipped))
    }
}

/// 把单插件投影结果构造成逐插件报告：成功即 `applied`（含已写文件与跳过项），
/// 失败即 `failed`（不回报任何文件与跳过项，只留原因）。
fn build_report(
    id: String,
    projected: Result<(Vec<String>, Vec<SkippedProvider>), String>,
) -> PluginApplyReport {
    match projected {
        Ok((files, skipped)) => PluginApplyReport {
            id,
            status: ApplyStatus::Applied,
            files,
            skipped,
            reason: None,
        },
        Err(reason) => PluginApplyReport {
            id,
            status: ApplyStatus::Failed,
            files: Vec::new(),
            skipped: Vec::new(),
            reason: Some(reason),
        },
    }
}

/// 未执行投影的插件报告（禁用或装载失败）：状态为 `skipped`，只携带原因。
fn skipped_report(id: String, reason: String) -> PluginApplyReport {
    PluginApplyReport {
        id,
        status: ApplyStatus::Skipped,
        files: Vec::new(),
        skipped: Vec::new(),
        reason: Some(reason),
    }
}

/// 把插件返回的相对路径拼成宿主绝对路径；越界路径一律报错（fail-loud）：
/// 绝对路径与 `..` 都指向预开放目录之外，不得被回报成一次成功投影。
pub(super) fn project_files(config_dir: &Path, files: Vec<String>) -> Result<Vec<String>, String> {
    files
        .into_iter()
        .map(|file| {
            let relative = Path::new(&file);
            if relative.is_absolute()
                || relative.components().any(|component| {
                    matches!(
                        component,
                        Component::RootDir | Component::Prefix(_) | Component::ParentDir
                    )
                })
            {
                return Err(format!(
                    "插件返回的文件路径「{file}」不是 config_dir 内的相对路径"
                ));
            }
            Ok(config_dir.join(relative).display().to_string())
        })
        .collect()
}
