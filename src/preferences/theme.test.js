import { describe, expect, it } from "vitest";
import { THEME_DARK, THEME_LIGHT, THEME_SYSTEM } from "@/api/preferences";
import { resolveAppearance } from "./theme";

describe("外观的解析", () => {
  it("选亮色即亮色，与系统外观无关", () => {
    expect(resolveAppearance(THEME_LIGHT, "light")).toBe("light");
    expect(resolveAppearance(THEME_LIGHT, "dark")).toBe("light");
  });

  it("选暗色即暗色，与系统外观无关", () => {
    expect(resolveAppearance(THEME_DARK, "light")).toBe("dark");
    expect(resolveAppearance(THEME_DARK, "dark")).toBe("dark");
  });

  it("跟随系统时取系统外观", () => {
    expect(resolveAppearance(THEME_SYSTEM, "dark")).toBe("dark");
    expect(resolveAppearance(THEME_SYSTEM, "light")).toBe("light");
  });

  it("系统外观缺省（读不到）时按亮色", () => {
    expect(resolveAppearance(THEME_SYSTEM, undefined)).toBe("light");
  });
});
