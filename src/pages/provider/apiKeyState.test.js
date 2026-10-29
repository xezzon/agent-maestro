import { describe, expect, it } from "vitest";
import { API_KEY_STATES } from "@/api/provider";
import { apiKeyStatePresentation } from "./apiKeyState";

describe("凭证状态的界面呈现", () => {
  it("三态各有一个标签", () => {
    expect(apiKeyStatePresentation("unset")).toEqual({
      label: "未设置",
      pendingMigration: false,
      hasCredential: false,
      placeholder: "留空则不设置",
    });
    expect(apiKeyStatePresentation("set")).toEqual({
      label: "已设置",
      pendingMigration: false,
      hasCredential: true,
      placeholder: "留空即清除当前密钥",
    });
    expect(apiKeyStatePresentation("plaintext")).toEqual({
      label: "已设置",
      pendingMigration: true,
      hasCredential: true,
      placeholder: "留空即清除当前密钥",
    });
  });

  it("明文态与已设置态标签相同：区别由「明文待迁移」告警点出", () => {
    expect(apiKeyStatePresentation("plaintext").label).toBe(
      apiKeyStatePresentation("set").label,
    );
    expect(
      apiKeyStatePresentation("plaintext").pendingMigration,
      "明文态要单独告警",
    ).toBe(true);
  });

  it("状态枚举的每个取值都有呈现（漏一个即静默不显示）", () => {
    for (const state of API_KEY_STATES) {
      expect(apiKeyStatePresentation(state)).not.toBeNull();
    }
    expect(API_KEY_STATES).toHaveLength(3);
  });

  it("未知状态不渲染标签", () => {
    expect(apiKeyStatePresentation("unknown")).toBeNull();
    expect(apiKeyStatePresentation(undefined)).toBeNull();
  });
});
