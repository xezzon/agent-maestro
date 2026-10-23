import { describe, expect, it } from "vitest";
import { urlRule } from "./urlRules.js";

/** 直接调用规则对象，不经过 DOM：antd 的 validator 签名是 (rule, value)。 */
function validate(rule, value) {
  return rule.validator(rule, value);
}

/** 跑完一条规则的 validator，返回拒绝原因（放行时为 null）。 */
async function rejectionOf(rule, value) {
  try {
    await validate(rule, value);
    return null;
  } catch (err) {
    return err.message;
  }
}

const HTTP_AND_HTTPS = urlRule({
  schemes: ["http:", "https:"],
  invalidUrl: "Base URL 不是合法的 URL",
  badScheme: "Base URL 仅支持 http(s) 地址",
});

const HTTPS_ONLY = urlRule({
  schemes: ["https:"],
  invalidUrl: "不是合法的 URL",
  badScheme: "插件来源仅支持 https 地址",
});

describe("URL 校验规则", () => {
  it("空值放行（必填由另一条规则管）", async () => {
    expect(await rejectionOf(HTTP_AND_HTTPS, "")).toBeNull();
    expect(await rejectionOf(HTTP_AND_HTTPS, undefined)).toBeNull();
  });

  it("非法 URL 按各自的文案拒绝", async () => {
    expect(await rejectionOf(HTTP_AND_HTTPS, "不是 URL")).toBe(
      "Base URL 不是合法的 URL",
    );
    expect(await rejectionOf(HTTPS_ONLY, "不是 URL")).toBe("不是合法的 URL");
  });

  it("协议白名单之外一律拒绝", async () => {
    expect(await rejectionOf(HTTP_AND_HTTPS, "ftp://example.com")).toBe(
      "Base URL 仅支持 http(s) 地址",
    );
    expect(await rejectionOf(HTTPS_ONLY, "http://example.com")).toBe(
      "插件来源仅支持 https 地址",
    );
  });

  it("白名单之内的协议放行", async () => {
    expect(await rejectionOf(HTTP_AND_HTTPS, "http://example.com")).toBeNull();
    expect(await rejectionOf(HTTP_AND_HTTPS, "https://example.com")).toBeNull();
    expect(await rejectionOf(HTTPS_ONLY, "https://example.com")).toBeNull();
  });
});
