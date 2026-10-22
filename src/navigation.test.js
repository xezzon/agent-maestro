import { describe, expect, it, vi } from "vitest";
import { currentPluginOf, pluginNavItems } from "./navigation.js";

// Tauri IPC 是系统边界：读取路径只能通过打桩 `invoke` 观察。
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

/**
 * 构造一个插件视图（界面形态）：未显式给出的字段取空形态。
 * @param {Partial<import("@/api/plugins").PluginView>} overrides
 */
function makePlugin(overrides = {}) {
  return {
    source: "builtin:pi",
    builtin: true,
    enabled: true,
    id: "pi",
    ...overrides,
  };
}

describe("侧边栏的导航派生", () => {
  it("只有可用插件（已启用且加载成功）占导航项", () => {
    const items = pluginNavItems([
      makePlugin(),
      makePlugin({ id: "off", enabled: false }),
      makePlugin({ id: "bad", error: "配置已损坏" }),
    ]);

    expect(items).toStrictEqual([{ key: "plugin:pi", label: "pi" }]);
  });

  it("没有可用插件时没有导航项", () => {
    expect(pluginNavItems([])).toStrictEqual([]);
    expect(
      pluginNavItems([makePlugin({ enabled: false })]),
      "禁用插件不进导航",
    ).toStrictEqual([]);
  });

  it("当前导航 key 指向插件时取到该插件", () => {
    const plugins = [
      makePlugin(),
      makePlugin({ id: "zed", source: "builtin:zed" }),
    ];

    expect(currentPluginOf(plugins, "plugin:zed")).toStrictEqual(plugins[1]);
    expect(currentPluginOf(plugins, "plugin:pi")).toStrictEqual(plugins[0]);
  });

  it("不是插件配置页、或插件已不在列表里时没有当前插件", () => {
    const plugins = [makePlugin()];

    expect(currentPluginOf(plugins, "providers")).toBeNull();
    expect(currentPluginOf(plugins, "plugins")).toBeNull();
    expect(currentPluginOf(plugins, "plugin:gone")).toBeNull();
    expect(currentPluginOf([], "plugin:pi")).toBeNull();
  });
});
