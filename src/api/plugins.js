/**
 * @typedef {Object} PluginView
 * @property {string} source 插件条目唯一身份（`builtin:<id>` 或 Git 仓库地址）。
 * @property {boolean} builtin 内置插件可禁用、不可移除、不出现在添加流程。
 * @property {boolean} enabled
 * @property {string=} id 安装目录名；尚未成功安装时缺省。
 * @property {string=} name
 * @property {string=} tool 适配的工具。
 * @property {string=} config_dir 插件被授权写入的目录（`~` 已展开）。
 * @property {'loaded' | 'error'} status
 * @property {string=} error 错误态的原因（网络 / manifest 不合法 / 接口不兼容 / id 冲突）。
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
 * 添加 Git 来源插件：命令返回前会先落配置条目，下载失败保留条目可重试。
 * @param {string} source 仅支持匿名 HTTPS 地址。
 */
export async function addPlugin(source) {
  await invoke("add_plugin", { source });
}

/**
 * 拉取最新版本；失败时旧版本继续可用。
 * @param {string} source
 */
export async function updatePlugin(source) {
  await invoke("update_plugin", { source });
}

/**
 * 配置条目与插件目录一并清理；内置插件会被后端拒绝。
 * @param {string} source
 */
export async function removePlugin(source) {
  await invoke("remove_plugin", { source });
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
