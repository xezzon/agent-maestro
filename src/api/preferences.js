/**
 * @typedef {'light' | 'dark' | 'system'} ThemePreference 主题偏好：亮色、暗色、跟随系统。
 * @typedef {'light' | 'dark'} Appearance 解析后的外观：跟随系统解析后的明暗。
 */
/**
 * 偏好文档（`~/.maestro/preferences.json` 的内容，后端 `Preferences` 的线格式）。
 * @typedef {Object} Preferences
 * @property {number=} version 偏好文件 schema 版本；写入时由后端强制，前端不必给。
 * @property {ThemePreference} theme 主题偏好。
 */
import { invoke } from "@tauri-apps/api/core";

/** @type {ThemePreference} 亮色：应用外观固定为亮色。 */
export const THEME_LIGHT = "light";
/** @type {ThemePreference} 暗色：应用外观固定为暗色。 */
export const THEME_DARK = "dark";
/** @type {ThemePreference} 跟随系统：应用外观随操作系统的明暗外观变化。 */
export const THEME_SYSTEM = "system";

/**
 * 读取偏好：返回整份偏好文档。
 *
 * 读不出来（存储锁不可用）即抛出原因——界面据此提示，而不是展示一份没有的偏好。
 * @returns {Promise<Preferences>}
 */
export async function getPreferences() {
  return invoke("get_preferences");
}

/**
 * 整包替换偏好：写盘失败即抛出原因（界面据此回滚到切换前的值，不展示一个
 * 没被保存的状态）。
 * @param {Preferences} preferences
 * @returns {Promise<Preferences>}
 */
export async function updatePreferences(preferences) {
  return invoke("update_preferences", { preferences });
}
