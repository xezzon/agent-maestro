import { getCurrentWindow } from "@tauri-apps/api/window";
import { THEME_SYSTEM } from "@/api/preferences";

/**
 * 是否运行在 Tauri 宿主里。
 *
 * 浏览器开发服务器（`pnpm dev`）下没有 Tauri 内部接口，窗口 API 不可用——
 * 此时「跟随系统」靠浏览器能力（matchMedia）工作，开发时也能验证。
 * @returns {boolean}
 */
export function inTauriHost() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/**
 * 深浅布尔 → 外观取值：亮色 / 暗色。
 *
 * 系统外观的各条来源（窗口事件、matchMedia）都只给出「是不是深色」，归一到
 * 外观取值只有这一处，免得几处各写一遍而漂移。
 * @param {boolean} isDark 是否深色。
 * @returns {import("@/api/preferences").Appearance}
 */
function toAppearance(isDark) {
  return isDark ? "dark" : "light";
}

/**
 * 系统外观：窗口 API 优先（宿主报告的操作系统外观），失败退回浏览器的
 * matchMedia。两者都解析成亮色 / 暗色。
 * @returns {Promise<import("@/api/preferences").Appearance>}
 */
export async function readSystemAppearance() {
  if (inTauriHost()) {
    try {
      const theme = await getCurrentWindow().theme();
      if (theme === "dark" || theme === "light") return theme;
    } catch (err) {
      // 宿主能力边界：无 portal 的 Linux 环境（无 D-Bus、部分 Flatpak 沙箱）收不到
      // 系统外观，宿主会兜底成亮色。只记 warn，不弹错。
      console.warn("failed to read the system theme from the window", err);
    }
  }
  return matchMediaAppearance();
}

/**
 * 浏览器能读到的系统外观：深色偏好即暗色。
 * @returns {import("@/api/preferences").Appearance}
 */
export function matchMediaAppearance() {
  if (
    typeof window === "undefined" ||
    typeof window.matchMedia !== "function"
  ) {
    return "light";
  }
  return toAppearance(
    window.matchMedia("(prefers-color-scheme: dark)").matches,
  );
}

/**
 * 订阅系统外观变化：窗口事件与 matchMedia 同时挂上，两者都解析成亮色 / 暗色。
 *
 * 两条来源互为补充：Tauri 宿主走窗口事件，浏览器开发服务器走 matchMedia。
 * 都收不到时「跟随系统」等价于固定亮色（宿主能力边界，见 ADR 0023）。
 * @param {(appearance: import("@/api/preferences").Appearance) => void} listener
 * @returns {() => void} 取消订阅
 */
export function subscribeSystemAppearance(listener) {
  const disposers = [];
  let cancelled = false;

  if (inTauriHost()) {
    getCurrentWindow()
      .onThemeChanged(({ payload }) => {
        listener(toAppearance(payload === "dark"));
      })
      .then((unlisten) => {
        // 订阅是异步的：取消订阅可能先于它完成，那时立即退订，不留悬挂监听。
        if (cancelled) {
          unlisten();
          return;
        }
        disposers.push(unlisten);
      })
      .catch((err) => console.warn("failed to watch the system theme", err));
  }

  if (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function"
  ) {
    const query = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = (event) => listener(toAppearance(event.matches));
    query.addEventListener("change", onChange);
    disposers.push(() => query.removeEventListener("change", onChange));
  }

  return () => {
    cancelled = true;
    disposers.forEach((dispose) => dispose());
  };
}

/**
 * 让原生标题栏与控件跟随应用外观：选亮色 / 暗色时显式设置，选跟随系统时传
 * `null` 交还系统。
 * @param {import("@/api/preferences").ThemePreference} preference 主题偏好。
 */
export function applyNativeTheme(preference) {
  if (!inTauriHost()) return;
  const theme = preference === THEME_SYSTEM ? null : preference;
  getCurrentWindow()
    .setTheme(theme)
    .catch((err) => console.warn("failed to set the native window theme", err));
}
