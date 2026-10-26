import { THEME_DARK, THEME_LIGHT } from "@/api/preferences";

/**
 * 解析后的外观：主题偏好 + 系统外观 → 亮色 / 暗色。
 *
 * 纯函数：不依赖 DOM 与 React，故推理与验证都不必起应用——「跟随系统」的语义
 * 全在这一句里（其余取值不依赖系统外观）。
 * @param {import("@/api/preferences").ThemePreference} preference 主题偏好。
 * @param {import("@/api/preferences").Appearance} systemAppearance 系统外观。
 * @returns {import("@/api/preferences").Appearance}
 */
export function resolveAppearance(preference, systemAppearance) {
  if (preference === THEME_LIGHT) return "light";
  if (preference === THEME_DARK) return "dark";
  return systemAppearance === "dark" ? "dark" : "light";
}
