import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  ANTHROPIC_MESSAGES,
  OPENAI_COMPLETIONS,
  configuredProtocols,
  initialApiKeyDraft,
  listProviders,
  normalizeEndpoints,
  normalizeModel,
  resolveApiKeyAction,
  toProviderPayload,
} from "./provider.js";

// Tauri IPC 是系统边界：读取路径的排序口径只能通过打桩 `invoke` 观察。
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

/**
 * 构造一条 Provider 视图（界面形态）：未显式给出的字段取空形态，
 * 让每个用例只写自己关心的那部分输入。`api_key_state` 是读取路径生成的
 * 只读字段，视图里恒存在（写入负载不得携带它）。
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
    api_key_state: "unset",
    ...overrides,
  };
}

/**
 * 构造一条后端记录（`list_providers` 的返回形态）。
 * @param {Partial<import("./provider.js").ProviderRecord>} overrides
 */
function makeRecord(overrides = {}) {
  return {
    enabled: true,
    base_url: {},
    selected_protocol: null,
    api_key_state: "unset",
    custom_header: {},
    models: [],
    ...overrides,
  };
}

describe("normalizeEndpoints", () => {
  it("missing_protocol_slot_is_filled_with_empty_string", () => {
    expect(
      normalizeEndpoints({
        "openai-completions": "https://api.example.com/v1",
      }),
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
    expect(
      normalizeModel({ id: "gpt-4o" }).limit,
      "limit 恒为对象",
    ).toStrictEqual({});
  });

  it("limit_subfield_with_empty_value_keeps_no_key", () => {
    expect(
      normalizeModel({
        id: "gpt-4o",
        limit: {
          context_window: null,
          max_input: 128000,
          max_output: undefined,
        },
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
    expect(
      payload.selected_protocol,
      "端点未填写则选择失效，归 null",
    ).toBeNull();
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

  it("enabled_state_is_passed_through_verbatim", () => {
    const payload = toProviderPayload(makeProvider({ enabled: false }), {
      kind: "keep",
    });

    expect(payload.enabled, "启用状态随整包回传").toBe(false);
  });

  it("payload_carries_only_the_write_fields", () => {
    const payload = toProviderPayload(makeProvider({ api_key_state: "set" }), {
      kind: "keep",
    });

    expect(
      Object.keys(payload).sort(),
      "写入负载的字段集合固定，视图专用字段不越界",
    ).toStrictEqual([
      "base_url",
      "custom_header",
      "enabled",
      "models",
      "selected_protocol",
      "slug",
    ]);
    expect(payload, "只读视图字段不进写入负载").not.toHaveProperty(
      "api_key_state",
    );
  });
});

describe("toProviderPayload 的密钥动作", () => {
  it("keep 的负载不带 api_key 字段：后端按「不变」处理", () => {
    const payload = toProviderPayload(makeProvider(), { kind: "keep" });

    expect(payload, "没动密钥就不带这个字段").not.toHaveProperty("api_key");
  });

  it("clear 的负载带空串：后端按「清除」处理", () => {
    const payload = toProviderPayload(makeProvider(), { kind: "clear" });

    expect(payload.api_key, "清除即空串").toBe("");
  });

  it("set 的负载带新值：后端按「覆盖」处理", () => {
    const payload = toProviderPayload(makeProvider(), {
      kind: "set",
      value: "sk-new",
    });

    expect(payload.api_key, "覆盖即新值").toBe("sk-new");
  });

  it("永不把引用当新值回传：负载里的凭证只来自用户此刻输入的值", () => {
    const payload = toProviderPayload(makeProvider(), {
      kind: "set",
      value: "sk-new",
    });

    expect(
      payload.api_key,
      "引用不是新值：视图里压根没有引用，前端也就无从回传它",
    ).toBe("sk-new");
  });
});

describe("initialApiKeyDraft", () => {
  it("未设置态直接摆出输入框", () => {
    expect(initialApiKeyDraft("unset"), "未设置态没有旧值可清").toStrictEqual({
      value: "",
    });
  });

  it("已设置 / 明文态先收着：点「更换 API Key」才展开", () => {
    expect(initialApiKeyDraft("set")).toBeNull();
    expect(initialApiKeyDraft("plaintext")).toBeNull();
  });
});

describe("resolveApiKeyAction", () => {
  it("未展开输入框 = 不碰密钥", () => {
    expect(resolveApiKeyAction("set", null), "已设置态未展开").toStrictEqual({
      kind: "keep",
    });
    expect(
      resolveApiKeyAction("plaintext", null),
      "明文态未展开",
    ).toStrictEqual({
      kind: "keep",
    });
  });

  it("未设置态留空 = 不碰密钥", () => {
    expect(
      resolveApiKeyAction("unset", { value: "" }),
      "未设置态没有旧值可清，留空即什么都不做",
    ).toStrictEqual({ kind: "keep" });
  });

  it("已设置态展开后留空 = 清除", () => {
    expect(
      resolveApiKeyAction("set", { value: "" }),
      "展开后留空即清除",
    ).toStrictEqual({
      kind: "clear",
    });
    expect(
      resolveApiKeyAction("plaintext", { value: "" }),
      "明文态同样可清除",
    ).toStrictEqual({ kind: "clear" });
  });

  it("有值 = 覆盖", () => {
    expect(resolveApiKeyAction("unset", { value: "sk-x" })).toStrictEqual({
      kind: "set",
      value: "sk-x",
    });
    expect(resolveApiKeyAction("set", { value: "sk-x" })).toStrictEqual({
      kind: "set",
      value: "sk-x",
    });
  });

  it("只有空白等同于留空：不会把空格当成新密钥覆盖上去", () => {
    expect(resolveApiKeyAction("set", { value: "   " })).toStrictEqual({
      kind: "clear",
    });
    expect(resolveApiKeyAction("unset", { value: "   " })).toStrictEqual({
      kind: "keep",
    });
  });

  it("新值去掉首尾空白", () => {
    expect(resolveApiKeyAction("unset", { value: " sk-x " })).toStrictEqual({
      kind: "set",
      value: "sk-x",
    });
  });
});

describe("listProviders", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("providers_are_sorted_by_byte_order", async () => {
    invokeMock.mockResolvedValue({
      beta: makeRecord(),
      alpha_2: makeRecord(),
      alpha2: makeRecord(),
      "alpha-2": makeRecord(),
      alpha: makeRecord(),
    });

    const providers = await listProviders();

    expect(
      providers.map((provider) => provider.slug),
      "按字节序（'-'=45 < '2'=50 < '_'=95），与记录层 BTreeMap 一致",
    ).toStrictEqual(["alpha", "alpha-2", "alpha2", "alpha_2", "beta"]);
  });

  it("custom_header_rows_are_sorted_by_name", async () => {
    invokeMock.mockResolvedValue({
      acme: makeRecord({
        custom_header: {
          "x-Alpha": "1",
          X_Alpha: "2",
          "X-Zeta": "3",
          "X-Alpha": "4",
        },
      }),
    });

    const [provider] = await listProviders();

    expect(
      provider.custom_header.map((header) => header.name),
      "header 行按名排序：字节序（'X'=88 早于 'x'=120，'-'=45 早于 '_'=95）",
    ).toStrictEqual(["X-Alpha", "X-Zeta", "X_Alpha", "x-Alpha"]);
  });

  it("api_key_state_tells_the_three_states_apart", async () => {
    invokeMock.mockResolvedValue({
      withKey: makeRecord({ api_key_state: "set" }),
      withoutKey: makeRecord({ api_key_state: "unset" }),
      legacy: makeRecord({ api_key_state: "plaintext" }),
    });

    const providers = await listProviders();
    const bySlug = Object.fromEntries(
      providers.map((provider) => [provider.slug, provider]),
    );

    expect(bySlug.withKey.api_key_state, "真值在密钥链里").toBe("set");
    expect(bySlug.withoutKey.api_key_state, "没有凭证").toBe("unset");
    expect(bySlug.legacy.api_key_state, "真值仍在配置文件里").toBe("plaintext");
  });

  it("视图里既没有凭证值也没有引用：单向写入", async () => {
    invokeMock.mockResolvedValue({
      acme: makeRecord({ api_key_state: "set", api_key: "sk-canary-7d1f" }),
    });

    const [provider] = await listProviders();

    expect(provider, "没有 api_key 字段").not.toHaveProperty("api_key");
    expect(
      JSON.stringify(provider),
      "凭证值不得出现在视图的任何字段",
    ).not.toContain("sk-canary-7d1f");
    expect(provider, "没有旧的只读标记").not.toHaveProperty("api_key_set");
  });
});

describe("已配置的协议", () => {
  it("只把填了端点的协议算作已配置", () => {
    expect(configuredProtocols({})).toStrictEqual([]);
    expect(configuredProtocols(undefined)).toStrictEqual([]);
    expect(
      configuredProtocols({ [OPENAI_COMPLETIONS]: "  " }),
      "只有空白视为未配置",
    ).toStrictEqual([]);
    expect(
      configuredProtocols({ [OPENAI_COMPLETIONS]: "https://api.example.com" }),
    ).toStrictEqual([OPENAI_COMPLETIONS]);
    expect(
      configuredProtocols({
        [OPENAI_COMPLETIONS]: "https://api.example.com",
        [ANTHROPIC_MESSAGES]: "https://anthropic.example.com",
      }),
      "按协议声明序返回",
    ).toStrictEqual([OPENAI_COMPLETIONS, ANTHROPIC_MESSAGES]);
  });

  it("写入负载只携带已配置的槽位", () => {
    const payload = toProviderPayload(
      makeProvider({
        base_url: {
          [OPENAI_COMPLETIONS]: "https://api.example.com",
          [ANTHROPIC_MESSAGES]: "  ",
        },
      }),
    );

    expect(payload.base_url).toStrictEqual({
      [OPENAI_COMPLETIONS]: "https://api.example.com",
    });
  });
});
