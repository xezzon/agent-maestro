//! 投影：把 Provider 写入各插件的配置目录，并把插件回报的相对路径拼成宿主绝对路径。

use std::{
    collections::BTreeMap,
    path::{Component, Path},
    sync::Arc,
};

use crate::{interpolate, paths::MaestroPaths, provider::Provider, store::Variables};

use super::{
    PluginApplyReport, PluginService, PluginState, config::read_plugin_config,
    execution::SkippedProvider,
};

impl PluginService {
    /// 对注册表中的每个插件执行投影，返回逐插件报告：
    /// Loaded 实例化组件后调用 `write_provider`；Disabled 与 Error 态不执行投影。
    /// 单个插件的失败不影响其它插件，失败原因写入该插件的报告。
    ///
    /// 传入的每个 Provider 都参与投影：是否禁用由调用方先行剔除（#94 在命令层
    /// `enabled_providers` 完成），本函数不感知 `enabled`。投影按插件读取插件配置
    /// （ADR 0018）：全局变量表 ⊕ 该插件的 `variables` 覆盖后插值（同一占位符可按
    /// 插件解析出不同值），`form` 段在组件实例化之前就位、经 `get-config` import
    /// 递送；先插值、后映射端点（ADR 0015）、端点选择见 ADR 0016。插件配置读取或
    /// 解析失败、任一字段插值失败即该插件整体 `failed`——不实例化、不调用插件、
    /// 不写任何文件。
    ///
    /// 插件执行时长不受应用控制：注册表锁只在快照阶段短暂持有（克隆 Arc 与
    /// 收集跳过态），wasm 实例化与调用全部在锁外进行，不阻塞其它命令。
    pub(crate) fn write_providers(
        &self,
        providers: &BTreeMap<String, Provider>,
        variables: &Variables,
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
            log::debug!("projecting providers: id={id}");
            let report = match (|| -> Result<(Vec<String>, Vec<SkippedProvider>), String> {
                // 按插件读取插件配置（ADR 0018）：读取或解析失败即整插件失败，不静默降级。
                let plugin_config = read_plugin_config(&MaestroPaths::get().plugin_dir(&id))?;
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
                interpolate::interpolate_providers(providers, &merged).and_then(|projected| {
                    loaded_plugin
                        .instantiate_component(&self.engine, form)
                        .and_then(|mut instantiated| instantiated.write_provider(&projected))
                })
            })() {
                // 插件返回的是相对 config_dir 的路径：拼成宿主绝对路径回报给界面。
                Ok((files, skipped)) => {
                    match project_files(&loaded_plugin.manifest.config_dir, files) {
                        Ok(files) => PluginApplyReport {
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
                    }
                }
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
