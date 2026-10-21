import { describe, expect, it } from "vitest";
import {
  normalizeEndpoints,
  normalizeModel,
  toProviderPayload,
} from "./provider.js";

/**
 * 构造一条 Provider 视图（界面形态）：未显式给出的字段取空形态，
 * 让每个用例只写自己关心的那部分输入。
 * @param {Partial<import("./provider.js").Provider>} overrides
 */
function makeProvider(overrides = {}) {
  return {
    slug: "acme",
    enabled: true,
    base_url: {},
    selected_protocol: null,
    custom_header: [],
    models: [],
    ...overrides,
  };
}

describe("normalizeEndpoints", () => {
  it("missing_protocol_slot_is_filled_with_empty_string", () => {
    expect(
      normalizeEndpoints({ "openai-completions": "https://api.example.com/v1" }),
      "缺键的协议槽位补空串，两个槽位恒存在",
    ).toStrictEqual({
      "openai-completions": "https://api.example.com/v1",
      "anthropic-messages": "",
    });
  });

  it("both_protocol_slots_missing_are_filled_with_empty_strings", () => {
    expect(normalizeEndpoints({}), "两个槽位都缺失时补两个空串").toStrictEqual({
      "openai-completions": "",
      "anthropic-messages": "",
    });
  });

  it("undefined_base_url_is_filled_with_two_empty_strings", () => {
    expect(normalizeEndpoints(undefined), "缺省输入补两个空串").toStrictEqual({
      "openai-completions": "",
      "anthropic-messages": "",
    });
  });
});

describe("normalizeModel", () => {
  it("limit_is_always_an_object", () => {
    expect(normalizeModel({ id: "gpt-4o" }).limit, "limit 恒为对象").toStrictEqual(
      {},
    );
  });

  it("limit_subfield_with_empty_value_keeps_no_key", () => {
    expect(
      normalizeModel({
        id: "gpt-4o",
        limit: { context_window: null, max_input: 128000, max_output: undefined },
      }).limit,
      "空即缺键：null 与 undefined 都不保留键",
    ).toStrictEqual({ max_input: 128000 });
  });

  it("capabilities_are_deduplicated_in_enum_declaration_order", () => {
    expect(
      normalizeModel({
        id: "gpt-4o",
        capabilities: ["thinking", "tool_use", "thinking", "image_in"],
      }).capabilities,
      "按声明序 tool_use → image_in → thinking",
    ).toStrictEqual(["tool_use", "image_in", "thinking"]);
  });

  it("capabilities_outside_the_enum_are_dropped", () => {
    expect(
      normalizeModel({
        id: "gpt-4o",
        capabilities: ["tool_use", "video_in"],
      }).capabilities,
      "取值受枚举约束，枚举外的取值丢弃",
    ).toStrictEqual(["tool_use"]);
  });

  it("absent_capabilities_normalize_to_empty_list", () => {
    expect(
      normalizeModel({ id: "gpt-4o" }).capabilities,
      "缺省的 capabilities 归一为空列表",
    ).toStrictEqual([]);
  });
});

describe("toProviderPayload", () => {
  it("only_filled_endpoint_slots_are_kept", () => {
    const payload = toProviderPayload(
      makeProvider({
        base_url: {
          "openai-completions": "https://api.example.com/v1",
          "anthropic-messages": "",
        },
      }),
    );

    expect(payload.base_url, "只保留已填写的端点槽位").toStrictEqual({
      "openai-completions": "https://api.example.com/v1",
    });
  });

  it("both_endpoint_slots_are_kept_when_both_are_filled", () => {
    const payload = toProviderPayload(
      makeProvider({
        base_url: {
          "openai-completions": "https://api.example.com/v1",
          "anthropic-messages": "https://api.example.com/anthropic",
        },
        selected_protocol: "anthropic-messages",
      }),
    );

    expect(payload.base_url, "两个端点都填写时两个槽位都保留").toStrictEqual({
      "openai-completions": "https://api.example.com/v1",
      "anthropic-messages": "https://api.example.com/anthropic",
    });
  });

  it("endpoint_filled_with_blanks_counts_as_unset", () => {
    const payload = toProviderPayload(
      makeProvider({
        base_url: {
          "openai-completions": "   ",
          "anthropic-messages": "",
        },
        selected_protocol: "openai-completions",
      }),
    );

    expect(payload.base_url, "只填空白的端点视为未填写").toStrictEqual({});
    expect(payload.selected_protocol, "端点未填写则选择失效，归 null").toBeNull();
  });

  it("selected_protocol_falls_back_to_null_when_its_endpoint_is_cleared", () => {
    const payload = toProviderPayload(
      makeProvider({
        base_url: {
          "openai-completions": "https://api.example.com/v1",
          "anthropic-messages": "",
        },
        selected_protocol: "anthropic-messages",
      }),
    );

    expect(payload.selected_protocol, "端点清空后选择失效，归 null").toBeNull();
  });

  it("selected_protocol_is_kept_when_its_endpoint_is_filled", () => {
    const payload = toProviderPayload(
      makeProvider({
        base_url: {
          "openai-completions": "https://api.example.com/v1",
          "anthropic-messages": "",
        },
        selected_protocol: "openai-completions",
      }),
    );

    expect(payload.selected_protocol, "指向已填写端点的选择原样保留").toBe(
      "openai-completions",
    );
  });

  it("custom_header_rows_are_restored_to_a_name_value_map", () => {
    const payload = toProviderPayload(
      makeProvider({
        custom_header: [
          { name: "X-Upper", value: "2" },
          { name: "x-lower", value: "1" },
        ],
      }),
    );

    expect(payload.custom_header, "行列表还原为名→值映射").toStrictEqual({
      "X-Upper": "2",
      "x-lower": "1",
    });
  });

  it("custom_header_is_sorted_by_byte_order_not_locale_compare", () => {
    const payload = toProviderPayload(
      makeProvider({
        custom_header: [
          { name: "x-lower", value: "1" },
          { name: "X-Upper", value: "2" },
        ],
      }),
    );

    expect(
      Object.keys(payload.custom_header),
      "按字节序（'X'=88 早于 'x'=120），与记录层 BTreeMap 一致",
    ).toStrictEqual(["X-Upper", "x-lower"]);
  });

  it("models_are_normalized_one_by_one", () => {
    const payload = toProviderPayload(
      makeProvider({
        models: [
          {
            id: "gpt-4o",
            display_name: "",
            limit: { context_window: null, max_output: 16384 },
            capabilities: ["thinking", "tool_use", "tool_use"],
          },
        ],
      }),
    );

    expect(payload.models, "模型逐条归一").toStrictEqual([
      {
        id: "gpt-4o",
        display_name: "",
        limit: { max_output: 16384 },
        capabilities: ["tool_use", "thinking"],
      },
    ]);
  });

  it("api_key_and_enabled_state_are_passed_through_verbatim", () => {
    const payload = toProviderPayload(
      makeProvider({ enabled: false, api_key: "sk-test" }),
    );

    expect(payload.api_key, "凭证随整包回传").toBe("sk-test");
    expect(payload.enabled, "启用状态随整包回传").toBe(false);
  });
});
