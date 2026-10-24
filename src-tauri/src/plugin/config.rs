//! 插件配置读写（落位目录 `<id>/config.json`，ADR 0018）：变量覆盖与插件私有表单。
//!
//! 文件不存在视为「无覆盖、无表单」，损坏则报错并给出可读原因，绝不静默降级。

use std::{fs, io::ErrorKind, path::Path};

use crate::{
    errors::user_message,
    store::{AppStore, PluginEntry},
};

use super::{INSTALL_LOCK_POISONED, PLACED_CONFIG, PluginConfig, PluginService, write_placed_file};

impl PluginService {
    /// 读取插件配置（ADR 0018）：返回该插件对全局变量的覆盖与私有表单数据。
    /// 配置文件不存在视为「无覆盖、无表单」——新装插件开箱即用；文件损坏则
    /// 报错并给出可读原因，绝不静默降级为空配置。
    pub(crate) fn get_config(
        &self,
        store: &AppStore,
        source: &str,
    ) -> Result<PluginConfig, String> {
        let entry = store
            .with_read(|s| s.plugin_by_source(source))
            .map_err(user_message)?;
        read_plugin_config(&self.paths.plugin_dir(&entry.id))
    }

    /// 写入插件配置（整包替换，ADR 0018）：`variables` 覆盖与 `form` 表单整体
    /// 覆盖到 `<id>/config.json`。覆盖的变量名必须已在全局变量表中声明（声明的
    /// 真相源是 `Config.variables`），未声明即报错——覆盖一个未声明的变量无意义，
    /// 且易掩盖拼写错误。落盘经同目录临时文件 + 原子改名；整个流程在
    /// `install_lock` 内独占，与安装/重载/移除互斥。变量值与表单内容不进日志
    /// （ADR 0008），日志只带 id。
    pub(crate) fn set_config(
        &self,
        store: &AppStore,
        source: &str,
        plugin_config: &PluginConfig,
    ) -> Result<(), String> {
        let _install_guard = self
            .install_lock
            .lock()
            .map_err(|_| INSTALL_LOCK_POISONED)?;

        // 短锁内完成条目读取与校验：声明的真相源就是当前全局变量表。
        let entry = store.with_read(|guard| -> Result<PluginEntry, String> {
            let variables = guard.variables_snapshot().map_err(user_message)?;
            for name in plugin_config.variables.keys() {
                if !variables.contains_key(name) {
                    return Err(format!(
                        "变量「{name}」未在全局变量表中声明，不能写入插件配置的覆盖项；请先在全局变量中声明。"
                    ));
                }
            }
            guard.plugin_by_source(source).map_err(user_message)
        })?;

        let plugin_dir = self.paths.plugin_dir(&entry.id);
        fs::create_dir_all(&plugin_dir).map_err(|e| format!("创建插件目录失败：{e}"))?;
        let raw = serde_json::to_string_pretty(plugin_config)
            .map_err(|e| format!("序列化插件配置失败：{e}"))?;
        write_placed_file(&plugin_dir, PLACED_CONFIG, format!("{raw}\n").as_bytes())?;
        log::debug!("plugin config written: id={}", entry.id);
        Ok(())
    }
}

/// 读取落位目录中的插件配置（ADR 0018）：文件不存在视为「无覆盖、无表单」，
/// 损坏则报错并给出可读原因，绝不静默降级为空配置。get-config 命令与投影路径
/// 共用同一份读取与失败语义。
pub(super) fn read_plugin_config(plugin_dir: &Path) -> Result<PluginConfig, String> {
    let path = plugin_dir.join(PLACED_CONFIG);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(PluginConfig::default()),
        Err(e) => {
            return Err(format!("读取插件配置失败：{}\n原因：{e}", path.display()));
        }
    };
    serde_json::from_str(&raw).map_err(|e| {
        format!(
            "插件配置已损坏：{}\n原因：{e}\n请修复或删除该文件后重试；在此之前该插件的配置不可用。",
            path.display()
        )
    })
}
