/**
 * @typedef {Object} PluginView
 * @property {string} source 插件条目唯一身份（第一期仅 `builtin:<id>`；Git / 本地来源见 issue #36）。
 * @property {boolean} builtin 内置插件可禁用、不可移除、不出现在添加流程。
 * @property {boolean} enabled
 * @property {string=} id 插件 id。
 * @property {string=} name
 * @property {string=} tool 适配的工具。
 * @property {string=} config_dir 插件被授权写入的目录（`~` 已展开）。
 * @property {'loaded' | 'error'} status
 * @property {string=} error 错误态的原因（如来源暂不支持 / manifest 不合法 / 接口不兼容）。
 */
/**
 * @typedef {Object} SkippedProvider
 * @property {string} slug
 * @property {string} reason 协议槽位为零或两个非空的跳过原因。
 */
/**
 * 逐插件投影报告。
 * @typedef {Object} PluginApplyReport
 * @property {string} source
 * @property {string=} id
 * @property {string=} name
 * @property {'applied' | 'failed' | 'skipped'} status
 *   skipped = 已禁用或加载失败（未执行投影）；failed = 插件执行了但返回错误。
 * @property {string[]} files 已写入文件的路径列表（相对 config_dir）。
 * @property {SkippedProvider[]} skipped
 * @property {string=} reason
 */
import { invoke } from "@tauri-apps/api/core";

/**
 * @returns {Promise<PluginView[]>}
 */
export async function listPlugins() {
  return invoke("list_plugins");
}

/**
 * @param {string} source
 * @param {boolean} enabled
 */
export async function setPluginEnabled(source, enabled) {
  await invoke("set_plugin_enabled", { source, enabled });
}

/** 从磁盘重建插件注册表，不联网。 */
export async function reloadPlugins() {
  await invoke("reload_plugins");
}

/**
 * 应用到工具：调用所有已启用且加载成功的插件执行投影。
 * @returns {Promise<PluginApplyReport[]>}
 */
export async function applyProviders() {
  return invoke("apply_providers");
}
