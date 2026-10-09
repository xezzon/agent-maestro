/**
 * @typedef {Object} PluginView
 * @property {string} source 插件条目唯一身份（`builtin:<id>`、指向 manifest.json 的 https URL 或本机绝对路径）。
 * @property {boolean} builtin 内置插件可禁用、不可移除。
 * @property {boolean} enabled
 * @property {string=} id 插件 id（条目在安装成功后写入；内置插件由启动补装），界面展示用它。
 * @property {string=} config_dir manifest 声明的写入目录，已解析为宿主绝对路径（`$HOME` 等变量已展开）。
 * @property {Object=} settings_schema manifest 内联的 JSON Schema（draft 2020-12，ADR 0017），
 *   描述 form 段的形状，原样透传不校验；缺省即该插件无需用户填写。
 * @property {string=} error 加载/启动失败的原因（如落位文件损坏 / manifest 不合法 / 接口不兼容）。
 */
/**
 * @typedef {Object} SkippedProvider
 * @property {string} slug
 * @property {string} reason 跳过原因（当前仅未配置任何协议端点）。
 */
/**
 * 逐插件投影报告。
 * @typedef {Object} PluginApplyReport
 * @property {string} id 插件 id。
 * @property {'applied' | 'failed' | 'skipped'} status
 *   skipped = 已禁用或加载失败（未执行投影）；failed = 插件执行了但返回错误。
 * @property {string[]} files 已写入文件的宿主绝对路径列表，第一个是主文件（可为空）。
 * @property {SkippedProvider[]} skipped
 * @property {string=} reason
 */
/**
 * 插件配置（ADR 0018）：该插件对全局变量的覆盖与它私有的表单数据，整包读写。
 * @typedef {Object} PluginConfig
 * @property {Record<string, string>} variables 变量覆盖：键必须是全局变量表中已声明的变量名（写入时校验）
 * @property {Object=} form 表单数据，形状由 manifest 的 settings_schema 声明（ADR 0017），宿主不解释
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
 * 添加插件：来源为指向 manifest.json 的 https 地址或本机绝对路径（本地调试），
 * 按来源获取 manifest 与 wasm、校验、落位并装载。
 * 安装成功才写入条目：失败直接抛出原因，不留下半成品条目。
 * @param {string} source 指向 manifest.json 的 https 地址或本机绝对路径
 */
export async function addPlugin(source) {
  await invoke("add_plugin", { source });
}

/**
 * 重新加载插件：按配置中的来源重新获取 manifest 与 wasm，逐文件覆盖落位产物
 * （写失败可能留下混合态，见 ADR 0018）。
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
 * 读取插件配置：配置不存在返回空配置（新装插件开箱即用），损坏则报错。
 * @param {string} source
 * @returns {Promise<PluginConfig>}
 */
export async function getPluginConfig(source) {
  return invoke("get_plugin_config", { source });
}

/**
 * 写入插件配置（整包替换，ADR 0018）：variables 覆盖与 form 表单整体覆盖，
 * 未在全局变量表中声明的变量名会被拒绝。只写配置，不触发投影。
 * @param {string} source
 * @param {PluginConfig} config
 */
export async function setPluginConfig(source, config) {
  await invoke("set_plugin_config", { source, config });
}

/**
 * 应用到工具：调用所有已启用且加载成功的插件执行投影。
 * @returns {Promise<PluginApplyReport[]>}
 */
export async function applyProviders() {
  return invoke("apply_providers");
}
