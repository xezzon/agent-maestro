/**
 * @typedef {Object} PluginView
 * @property {string} source 插件条目唯一身份（`builtin:<id>` 或指向 manifest.json 的 https URL）。
 * @property {boolean} builtin 内置插件可禁用、不可移除、不出现在添加流程。
 * @property {boolean} enabled
 * @property {string=} id 插件 id（https 条目在安装成功后写入）。
 * @property {string=} name
 * @property {string=} tool 适配的工具。
 * @property {string=} config_dir 插件被授权写入的目录（`~` 已展开）。
 * @property {'loaded' | 'error'} status
 * @property {string=} error 错误态的原因（如下载/校验失败 / manifest 不合法 / 接口不兼容）。
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

/**
 * 添加插件：https 来源指向 manifest.json，下载、校验、落位并装载。
 * 条目一旦写入即保留：安装失败进错误态，用「重新加载」重试。
 * @param {string} url 指向 manifest.json 的 https 地址
 */
export async function addPlugin(url) {
  await invoke("add_plugin", { url });
}

/**
 * 重新加载插件：按配置中的来源重新获取 manifest 与 wasm，成功才替换旧版本
 * （失败时旧版本保持可用）。
 * @param {string} source
 */
export async function reloadPlugin(source) {
  await invoke("reload_plugin", { source });
}

/**
 * 移除插件：删配置条目与落位目录（幂等）。
 * @param {string} source
 */
export async function removePlugin(source) {
  await invoke("remove_plugin", { source });
}

/**
 * 应用到工具：调用所有已启用且加载成功的插件执行投影。
 * @returns {Promise<PluginApplyReport[]>}
 */
export async function applyProviders() {
  return invoke("apply_providers");
}
