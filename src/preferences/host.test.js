import { describe, expect, it, vi } from "vitest";

// Tauri 宿主是系统边界：窗口 API 只能通过打桩观察。
const host = vi.hoisted(() => ({
  theme: vi.fn(),
  setTheme: vi.fn(),
  onThemeChanged: vi.fn(),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => host,
}));

import {
  applyNativeTheme,
  readSystemAppearance,
  subscribeSystemAppearance,
} from "./host";

/**
 * 造一个 matchMedia 桩：记录订阅者，可手动触发变化。
 * @param {boolean} isDark 深色偏好。
 */
function makeMatchMedia(isDark) {
  const listeners = new Set();
  return {
    matches: isDark,
    media: "(prefers-color-scheme: dark)",
    addEventListener: vi.fn((_type, listener) => listeners.add(listener)),
    removeEventListener: vi.fn((_type, listener) => listeners.delete(listener)),
    /** 触发一次媒体查询变化。 */
    change(matches) {
      this.matches = matches;
      listeners.forEach((listener) => listener({ matches }));
    },
  };
}

/**
 * 装一个浏览器环境（有 matchMedia，没有 Tauri 内部接口）。
 * @param {boolean} isDark 深色偏好。
 */
function useBrowser(isDark) {
  const media = makeMatchMedia(isDark);
  globalThis.window = { matchMedia: () => media };
  return media;
}

/**
 * 装一个 Tauri 宿主环境（有内部接口，网页媒体查询由宿主管着）。
 * @param {boolean} isDark 网页媒体查询报出的深色偏好。
 */
function useHost(isDark) {
  const media = makeMatchMedia(isDark);
  globalThis.window = { __TAURI_INTERNALS__: {}, matchMedia: () => media };
  return media;
}

describe("系统外观的读取", () => {
  it("宿主里取窗口主题，不看网页的媒体查询", async () => {
    // 网页媒体查询报亮色：宿主里它跟着应用自己的选择走，不是系统外观。
    useHost(false);
    host.theme.mockResolvedValue("dark");

    expect(await readSystemAppearance()).toBe("dark");
  });

  it("宿主里窗口主题读不到时退回媒体查询", async () => {
    useHost(true);
    host.theme.mockRejectedValue(new Error("no window"));
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});

    expect(await readSystemAppearance()).toBe("dark");

    warn.mockRestore();
  });

  it("浏览器里取媒体查询", async () => {
    useBrowser(true);

    expect(await readSystemAppearance()).toBe("dark");
  });
});

describe("系统外观的订阅", () => {
  it("宿主里主题变化的通知不采信取值，改读当前外观", async () => {
    useHost(false);
    host.theme.mockResolvedValue("dark");
    let notify;
    host.onThemeChanged.mockImplementation((listener) => {
      notify = listener;
      return Promise.resolve(() => {});
    });
    const listener = vi.fn();

    subscribeSystemAppearance(listener);
    // 交还窗口主题时宿主会补一条「亮色」通知：取值会骗人，重读才是准的。
    notify({ payload: "light" });

    await vi.waitFor(() => expect(listener).toHaveBeenCalledWith("dark"));
  });

  it("宿主里网页媒体查询的变化不算系统外观", () => {
    const media = useHost(false);
    host.onThemeChanged.mockResolvedValue(() => {});

    subscribeSystemAppearance(vi.fn());

    expect(media.addEventListener).not.toHaveBeenCalled();
  });

  it("浏览器里媒体查询的变化即系统外观", () => {
    const media = useBrowser(false);
    const listener = vi.fn();

    subscribeSystemAppearance(listener);
    media.change(true);

    expect(listener).toHaveBeenCalledWith("dark");
  });
});

describe("原生主题的应用", () => {
  it("选亮色或暗色时显式设置窗口主题", async () => {
    useHost(false);
    host.setTheme.mockResolvedValue(undefined);

    await applyNativeTheme("dark");

    expect(host.setTheme).toHaveBeenCalledWith("dark");
  });

  it("跟随系统时把窗口主题交还系统", async () => {
    useHost(false);
    host.setTheme.mockResolvedValue(undefined);

    await applyNativeTheme("system");

    expect(host.setTheme).toHaveBeenCalledWith(null);
  });

  it("浏览器里没有宿主，什么都不做", async () => {
    useBrowser(false);

    await applyNativeTheme("dark");

    expect(host.setTheme).not.toHaveBeenCalled();
  });
});
