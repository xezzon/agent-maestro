import { describe, expect, it, vi } from "vitest";
import { isUsablePlugin } from "./plugins.js";

// Tauri IPC 是系统边界：读取路径只能通过打桩 `invoke` 观察。
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

/**
 * 构造一个插件视图（界面形态）：未显式给出的字段取空形态，
 * 让每个用例只写自己关心的那部分输入。
 * @param {Partial<import("./plugins.js").PluginView>} overrides
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

describe("插件是否可用", () => {
  it("已启用且加载成功才算可用", () => {
    expect(isUsablePlugin(makePlugin())).toBe(true);
    expect(isUsablePlugin(makePlugin({ enabled: false }))).toBe(false);
    expect(isUsablePlugin(makePlugin({ error: "配置已损坏" }))).toBe(false);
    expect(
      isUsablePlugin(makePlugin({ enabled: false, error: "配置已损坏" })),
    ).toBe(false);
  });
});
