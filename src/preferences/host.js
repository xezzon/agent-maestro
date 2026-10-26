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
 * 媒体的深浅偏好只给出「是不是深色」，归一到外观取值只有这一处，免得几处各写
 * 一遍而漂移。
 * @param {boolean} isDark 是否深色。
 * @returns {import("@/api/preferences").Appearance}
 */
function toAppearance(isDark) {
  return isDark ? "dark" : "light";
}

/**
 * 系统外观：宿主报告的操作系统外观。
 *
 * 宿主里读**窗口主题**（`window.theme()`），不读网页的 `prefers-color-scheme`：
 * 宿主自己管着 webview 的配色（Linux 上它还会改 GTK 的 prefer-dark 标记），
 * 网页里的媒体查询跟着**应用自己的选择**走，不是系统外观——拿它当系统外观，
 * 读到的就是被应用自己改过的值（选「跟随系统」时宿主把窗口主题交还系统，
 * 那一步顺手把媒体查询拨成了亮色）。
 *
 * 浏览器开发服务器没有宿主，才退回浏览器的 matchMedia。
 * @returns {Promise<import("@/api/preferences").Appearance>}
 */
export async function readSystemAppearance() {
  if (inTauriHost()) {
    try {
      const theme = await getCurrentWindow().theme();
      if (theme === "dark" || theme === "light") return theme;
    } catch (err) {
      // 宿主能力边界：窗口主题读不到（宿主接口不可用）时只剩浏览器的媒体查询
      // 可用。只记 warn，不弹错。
      console.warn("failed to read the system theme from the window", err);
    }
  }
  return matchMediaAppearance();
}

/**
 * 浏览器能读到的系统外观：深色偏好即暗色。
 *
 * 浏览器开发服务器里这是系统外观的来源；宿主里只作窗口主题读不到时的兜底——
 * 宿主里媒体查询跟着应用自己的选择走，多半读不到系统外观。
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
 * 订阅系统外观变化。
 *
 * 宿主里收到「主题变了」的通知就**重读一遍**当前外观，而不是直接采信通知里的
 * 取值：把窗口主题交还系统时宿主会补一条「亮色」通知（Linux 上把「交还」当成
 * 一次亮色切换），通知里的取值会骗人，读到的才是当前外观。
 *
 * 浏览器开发服务器没有宿主，订阅 matchMedia 的变化即可。
 * @param {(appearance: import("@/api/preferences").Appearance) => void} listener
 * @returns {() => void} 取消订阅
 */
export function subscribeSystemAppearance(listener) {
  const disposers = [];
  let cancelled = false;

  if (inTauriHost()) {
    getCurrentWindow()
      .onThemeChanged(() => {
        readSystemAppearance()
          .then(listener)
          .catch((err) => console.warn("failed to read the system theme", err));
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
  } else if (
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
 *
 * 交还之后窗口主题不再由应用定住，宿主报出的外观才是系统外观——「跟随系统」
 * 的外观要在这之后才读得到准，故调用方要等它完成再读。
 * @param {import("@/api/preferences").ThemePreference} preference 主题偏好。
 * @returns {Promise<void>}
 */
export async function applyNativeTheme(preference) {
  if (!inTauriHost()) return;
  const theme = preference === THEME_SYSTEM ? null : preference;
  try {
    await getCurrentWindow().setTheme(theme);
  } catch (err) {
    console.warn("failed to set the native window theme", err);
  }
}
