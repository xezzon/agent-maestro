/**
 * 全局变量表（ADR 0015）：名 → 默认值，随全局配置落盘。
 * 变量属全局配置，与 Provider 同页维护（issue #90）；配置值中的占位符
 * （`${NAME}` / `$NAME` / `${NAME:default}`）在投影时由宿主插值。
 * @typedef {Record<string, string>} Variables
 */
import { invoke } from "@tauri-apps/api/core";

/**
 * @returns {Promise<Variables>}
 */
export async function listVariables() {
  return invoke("list_variables");
}

/**
 * 整包替换全局变量表：新增、改名与删除都整表回传。
 * @param {Variables} variables
 */
export async function setVariables(variables) {
  await invoke("set_variables", { variables });
}
