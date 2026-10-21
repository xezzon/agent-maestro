import { describe, expect, it } from "vitest";
import {
  normalizeEndpoints,
  normalizeModel,
  toProviderPayload,
} from "./provider.js";

describe("normalizeEndpoints", () => {
  it("缺键的协议槽位补为空串，两个槽位恒存在", () => {
    expect(
      normalizeEndpoints({ "openai-completions": "https://api.example.com/v1" }),
    ).toEqual({
      "openai-completions": "https://api.example.com/v1",
      "anthropic-messages": "",
    });
  });

  it("两个协议槽位都缺失时补成两个空串", () => {
    expect(normalizeEndpoints({})).toEqual({
      "openai-completions": "",
      "anthropic-messages": "",
    });
  });

  it("缺省输入补成两个空串", () => {
    expect(normalizeEndpoints(undefined)).toEqual({
      "openai-completions": "",
      "anthropic-messages": "",
    });
  });
});

describe("normalizeModel", () => {
  it("limit 恒为对象，缺省时为空对象", () => {
    expect(normalizeModel({ id: "gpt-4o" }).limit).toStrictEqual({});
  });

  it("limit 里取空的子项不保留键", () => {
    expect(
      normalizeModel({
        id: "gpt-4o",
        limit: { context_window: null, max_input: 128000, max_output: undefined },
      }).limit,
    ).toStrictEqual({ max_input: 128000 });
  });

  it("capabilities 按枚举声明序去重排序", () => {
    expect(
      normalizeModel({
        id: "gpt-4o",
        capabilities: ["thinking", "tool_use", "thinking", "image_in"],
      }).capabilities,
    ).toEqual(["tool_use", "image_in", "thinking"]);
  });

  it("capabilities 只保留枚举内的取值", () => {
    expect(
      normalizeModel({
        id: "gpt-4o",
        capabilities: ["tool_use", "video_in"],
      }).capabilities,
    ).toEqual(["tool_use"]);
  });

  it("缺省的 capabilities 归一为空列表", () => {
    expect(normalizeModel({ id: "gpt-4o" }).capabilities).toEqual([]);
  });
});

describe("toProviderPayload", () => {
  it("只保留已填写的端点槽位", () => {
    const payload = toProviderPayload({
      slug: "acme",
      base_url: {
        "openai-completions": "https://api.example.com/v1",
        "anthropic-messages": "",
      },
      selected_protocol: null,
      custom_header: [],
      models: [],
    });

    expect(payload.base_url).toEqual({
      "openai-completions": "https://api.example.com/v1",
    });
  });

  it("两个端点都填写时两个槽位都保留", () => {
    const payload = toProviderPayload({
      slug: "acme",
      base_url: {
        "openai-completions": "https://api.example.com/v1",
        "anthropic-messages": "https://api.example.com/anthropic",
      },
      selected_protocol: "anthropic-messages",
      custom_header: [],
      models: [],
    });

    expect(payload.base_url).toEqual({
      "openai-completions": "https://api.example.com/v1",
      "anthropic-messages": "https://api.example.com/anthropic",
    });
  });

  it("端点清空后 selected_protocol 归 null", () => {
    const payload = toProviderPayload({
      slug: "acme",
      base_url: {
        "openai-completions": "https://api.example.com/v1",
        "anthropic-messages": "",
      },
      selected_protocol: "anthropic-messages",
      custom_header: [],
      models: [],
    });

    expect(payload.selected_protocol).toBeNull();
  });

  it("只填空白的端点视为未填写，selected_protocol 归 null", () => {
    const payload = toProviderPayload({
      slug: "acme",
      base_url: {
        "openai-completions": "   ",
        "anthropic-messages": "",
      },
      selected_protocol: "openai-completions",
      custom_header: [],
      models: [],
    });

    expect(payload.base_url).toStrictEqual({});
    expect(payload.selected_protocol).toBeNull();
  });

  it("selected_protocol 指向已填写的端点时原样保留", () => {
    const payload = toProviderPayload({
      slug: "acme",
      base_url: {
        "openai-completions": "https://api.example.com/v1",
        "anthropic-messages": "",
      },
      selected_protocol: "openai-completions",
      custom_header: [],
      models: [],
    });

    expect(payload.selected_protocol).toBe("openai-completions");
  });

  it("header 行列表还原为名→值映射", () => {
    const payload = toProviderPayload({
      slug: "acme",
      base_url: {},
      selected_protocol: null,
      custom_header: [
        { name: "X-Upper", value: "2" },
        { name: "x-lower", value: "1" },
      ],
      models: [],
    });

    expect(payload.custom_header).toStrictEqual({ "X-Upper": "2", "x-lower": "1" });
  });

  it("header 按字节序排序而非 localeCompare", () => {
    const payload = toProviderPayload({
      slug: "acme",
      base_url: {},
      selected_protocol: null,
      custom_header: [
        { name: "x-lower", value: "1" },
        { name: "X-Upper", value: "2" },
      ],
      models: [],
    });

    expect(Object.keys(payload.custom_header)).toEqual(["X-Upper", "x-lower"]);
  });

  it("模型逐条归一", () => {
    const payload = toProviderPayload({
      slug: "acme",
      base_url: {},
      selected_protocol: null,
      custom_header: [],
      models: [
        {
          id: "gpt-4o",
          display_name: "",
          limit: { context_window: null, max_output: 16384 },
          capabilities: ["thinking", "tool_use", "tool_use"],
        },
      ],
    });

    expect(payload.models).toStrictEqual([
      {
        id: "gpt-4o",
        display_name: "",
        limit: { max_output: 16384 },
        capabilities: ["tool_use", "thinking"],
      },
    ]);
  });

  it("凭证与启用状态原样回传", () => {
    const payload = toProviderPayload({
      slug: "acme",
      enabled: false,
      base_url: {},
      selected_protocol: null,
      api_key: "sk-test",
      custom_header: [],
      models: [],
    });

    expect(payload.api_key).toBe("sk-test");
    expect(payload.enabled).toBe(false);
  });
});
