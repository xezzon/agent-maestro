import { describe, expect, it } from "vitest";
import {
  APPLY_STATUS_APPLIED,
  APPLY_STATUS_FAILED,
  APPLY_STATUS_SKIPPED,
  APPLY_STATUSES,
} from "@/api/plugins";
import { applyStatusPresentation } from "./applyStatus";

describe("投影状态的界面呈现", () => {
  it("三个状态各有一个标签", () => {
    expect(applyStatusPresentation(APPLY_STATUS_APPLIED)).toEqual({
      label: "已写入",
      color: "success",
    });
    expect(applyStatusPresentation(APPLY_STATUS_FAILED)).toEqual({
      label: "失败",
      color: "error",
    });
    expect(applyStatusPresentation(APPLY_STATUS_SKIPPED)).toEqual({
      label: "已跳过",
      color: undefined,
    });
  });

  it("状态枚举的每个取值都有标签（漏一个即静默不显示）", () => {
    for (const status of APPLY_STATUSES) {
      expect(applyStatusPresentation(status)).not.toBeNull();
    }
    expect(APPLY_STATUSES).toHaveLength(3);
  });

  it("未知状态不渲染标签", () => {
    expect(applyStatusPresentation("unknown")).toBeNull();
    expect(applyStatusPresentation(undefined)).toBeNull();
  });
});
