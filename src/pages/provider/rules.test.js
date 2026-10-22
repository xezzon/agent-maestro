import { describe, expect, it } from "vitest";
import { OPENAI_COMPLETIONS } from "../../api/provider";
import {
  BASE_URL_RULES,
  HEADER_NAME_RULES,
  SLUG_RULES,
  atLeastOneEndpointRule,
  headerNameUniquenessRule,
  modelIdRule,
  providerFormRules,
  selectedProtocolRule,
  slugUniquenessRule,
} from "./rules.js";

/**
 * 构造 antd 表单依赖的最小替身：`getFieldValue` 同时支持
 * `"models"` 与 `["base_url", protocol]` 两种路径形态。
 * @param {Record<string, any>} values
 */
function makeForm(values) {
  return {
    getFieldValue: (path) => {
      const keys = Array.isArray(path) ? path : String(path).split(".");
      return keys.reduce((node, key) => node?.[key], values);
    },
  };
}

/** 直接调用规则对象，不经过 DOM：antd 的 validator 签名是 (rule, value)。 */
function validate(rule, value) {
  return rule.validator(rule, value);
}

/** 取出规则组里带 pattern 的那条（格式规则本身不是函数，直接测它的正则）。 */
function patternRule(rules) {
  const rule = rules.find((candidate) => candidate.pattern);
  expect(rule, "规则组里应有一条 pattern 规则").toBeDefined();
  return rule;
}

describe("slug 格式", () => {
  it("valid_slugs_pass_the_format_rule", () => {
    const { pattern } = patternRule(SLUG_RULES);

    expect(
      ["a", "acme", "acme-2", "acme_2", "a_b-c9"].every((slug) =>
        pattern.test(slug),
      ),
      "小写字母开头，允许小写字母、数字、连字符与下划线",
    ).toBe(true);
  });

  it("invalid_slugs_fail_the_format_rule", () => {
    const { pattern } = patternRule(SLUG_RULES);

    expect(
      ["Acme", "2acme", "-acme", "_acme", "ac me", "acme!", ""].filter((slug) =>
        pattern.test(slug),
      ),
      "大写、数字/连字符/下划线开头、空格、非法字符与空串都不符合 slug 格式",
    ).toStrictEqual([]);
  });

  it("slug_is_required", () => {
    expect(
      SLUG_RULES.some((rule) => rule.required === true),
      "slug 必填",
    ).toBe(true);
  });
});

describe("slug 唯一性", () => {
  const providers = [{ slug: "acme" }, { slug: "other" }];

  it("new_slug_that_collides_with_an_existing_provider_is_rejected", async () => {
    const rule = slugUniquenessRule({ providers, currentSlug: "" });

    await expect(validate(rule, "acme")).rejects.toThrow("已被使用");
  });

  it("new_slug_that_is_not_taken_passes", async () => {
    const rule = slugUniquenessRule({ providers, currentSlug: "" });

    await expect(validate(rule, "fresh")).resolves.toBeUndefined();
  });

  it("update_mode_skips_the_conflict_check", async () => {
    // 更新态由 ProviderCard 传入空列表：slug 不可编辑，行为保持不变。
    const rule = slugUniquenessRule({ providers: [], currentSlug: "acme" });

    await expect(validate(rule, "acme")).resolves.toBeUndefined();
  });

  it("the_provider_under_edit_does_not_conflict_with_itself", async () => {
    const rule = slugUniquenessRule({ providers, currentSlug: "acme" });

    await expect(validate(rule, "acme")).resolves.toBeUndefined();
  });
});

describe("端点 URL", () => {
  it("an_empty_endpoint_passes", async () => {
    const [rule] = BASE_URL_RULES;

    await expect(validate(rule, "")).resolves.toBeUndefined();
  });

  it("a_non_url_is_rejected", async () => {
    const [rule] = BASE_URL_RULES;

    await expect(validate(rule, "not a url")).rejects.toThrow("不是合法的 URL");
  });

  it("only_http_and_https_schemes_are_accepted", async () => {
    const [rule] = BASE_URL_RULES;

    await expect(validate(rule, "ftp://api.example.com")).rejects.toThrow(
      "仅支持 http(s)",
    );
    await expect(
      validate(rule, "https://api.example.com/v1"),
    ).resolves.toBeUndefined();
  });
});

describe("至少填一个端点", () => {
  it("both_protocol_slots_empty_is_rejected", async () => {
    const form = makeForm({
      base_url: { "openai-completions": "", "anthropic-messages": "" },
    });

    await expect(validate(atLeastOneEndpointRule(form), "")).rejects.toThrow(
      "请至少填写一个协议的 Base URL",
    );
  });

  it("whitespace_only_endpoints_count_as_unset", async () => {
    const form = makeForm({
      base_url: { "openai-completions": "   ", "anthropic-messages": "" },
    });

    await expect(validate(atLeastOneEndpointRule(form), "")).rejects.toThrow(
      "请至少填写一个协议的 Base URL",
    );
  });

  it("one_filled_endpoint_passes", async () => {
    const form = makeForm({
      base_url: {
        "openai-completions": "",
        "anthropic-messages": "https://api.anthropic.com",
      },
    });

    await expect(
      validate(atLeastOneEndpointRule(form), ""),
    ).resolves.toBeUndefined();
  });
});

describe("两个协议都配置时必须选定投影协议", () => {
  const bothFilled = {
    base_url: {
      "openai-completions": "https://api.example.com/v1",
      "anthropic-messages": "https://api.anthropic.com",
    },
  };

  it("no_selection_is_required_when_only_one_protocol_is_configured", async () => {
    const form = makeForm({
      base_url: { "openai-completions": "https://api.example.com/v1" },
    });

    await expect(
      validate(selectedProtocolRule(form), undefined),
    ).resolves.toBeUndefined();
  });

  it("a_configured_protocol_can_be_selected", async () => {
    const rule = selectedProtocolRule(makeForm(bothFilled));

    await expect(
      validate(rule, "anthropic-messages"),
    ).resolves.toBeUndefined();
  });

  it("an_empty_selection_is_rejected_when_both_protocols_are_configured", async () => {
    const rule = selectedProtocolRule(makeForm(bothFilled));

    await expect(validate(rule, undefined)).rejects.toThrow(
      "请选择投影使用的端点",
    );
  });

  it("a_value_that_is_not_a_configured_protocol_is_rejected", async () => {
    const rule = selectedProtocolRule(makeForm(bothFilled));

    await expect(validate(rule, "some-other-protocol")).rejects.toThrow(
      "请选择投影使用的端点",
    );
  });
});

describe("header 名", () => {
  it("http_token_characters_pass_the_format_rule", () => {
    const { pattern } = patternRule(HEADER_NAME_RULES);

    expect(
      ["anthropic-version", "X_Custom", "x!$%&'*+-.^_`|~"].every((name) =>
        pattern.test(name),
      ),
      "HTTP token（tchar）字符集内的名字合法",
    ).toBe(true);
  });

  it("names_outside_the_http_token_charset_fail_the_format_rule", () => {
    const { pattern } = patternRule(HEADER_NAME_RULES);

    expect(
      ["x-api-key: v", "with space", "naïve", "a/b", ""].filter((name) =>
        pattern.test(name),
      ),
      "空格、冒号、非 ASCII 字符与空串都不是合法 token",
    ).toStrictEqual([]);
  });

  it("header_name_is_required", () => {
    expect(
      HEADER_NAME_RULES.some((rule) => rule.required === true),
      "header 名必填",
    ).toBe(true);
  });
});

describe("header 名唯一性", () => {
  it("a_case_insensitively_duplicated_name_is_rejected", async () => {
    const form = makeForm({
      custom_header: [{ name: "X-Api-Key" }, { name: "x-api-key" }],
    });

    await expect(
      validate(headerNameUniquenessRule(form), "X-API-KEY"),
    ).rejects.toThrow("在当前 Provider 内重复（忽略大小写）");
  });

  it("a_unique_name_passes", async () => {
    const form = makeForm({
      custom_header: [{ name: "anthropic-version" }, { name: "x-api-key" }],
    });

    await expect(
      validate(headerNameUniquenessRule(form), "x-api-key"),
    ).resolves.toBeUndefined();
  });

  it("an_empty_name_is_left_to_the_required_rule", async () => {
    const form = makeForm({
      custom_header: [{ name: "" }, { name: "" }],
    });

    await expect(
      validate(headerNameUniquenessRule(form), ""),
    ).resolves.toBeUndefined();
  });
});

describe("模型 ID", () => {
  it("an_empty_or_whitespace_id_is_rejected", async () => {
    const form = makeForm({ models: [] });

    await expect(validate(modelIdRule(form), "")).rejects.toThrow(
      "请输入模型 ID",
    );
    await expect(validate(modelIdRule(form), "   ")).rejects.toThrow(
      "请输入模型 ID",
    );
  });

  it("a_duplicated_id_is_rejected", async () => {
    const form = makeForm({
      models: [{ id: "gpt-4o" }, { id: "gpt-4o" }],
    });

    await expect(validate(modelIdRule(form), "gpt-4o")).rejects.toThrow(
      "在当前 Provider 内重复",
    );
  });

  it("a_unique_id_passes", async () => {
    const form = makeForm({
      models: [{ id: "gpt-4o" }, { id: "gpt-4o-mini" }],
    });

    await expect(
      validate(modelIdRule(form), "gpt-4o-mini"),
    ).resolves.toBeUndefined();
  });

  it("the_uniqueness_check_is_case_sensitive", async () => {
    const form = makeForm({
      models: [{ id: "GPT-4o" }, { id: "gpt-4o" }],
    });

    await expect(
      validate(modelIdRule(form), "gpt-4o"),
    ).resolves.toBeUndefined();
  });
});

describe("providerFormRules", () => {
  it("live_checks_are_wired_into_their_field_groups", async () => {
    const form = makeForm({
      base_url: { "openai-completions": "", "anthropic-messages": "" },
      custom_header: [{ name: "x-api-key" }, { name: "X-API-Key" }],
      models: [{ id: "gpt-4o" }, { id: "gpt-4o" }],
    });
    const rules = providerFormRules({
      form,
      providers: [{ slug: "acme" }],
      currentSlug: "",
    });

    await expect(validate(rules.slug.at(-1), "acme")).rejects.toThrow(
      "已被使用",
    );
    await expect(validate(rules.baseUrl[OPENAI_COMPLETIONS].at(-1), "")).rejects.toThrow(
      "请至少填写一个协议的 Base URL",
    );
    await expect(validate(rules.headerName.at(-1), "x-api-key")).rejects.toThrow(
      "重复",
    );
    await expect(validate(rules.modelId.at(-1), "gpt-4o")).rejects.toThrow(
      "重复",
    );
  });

  it("selected_protocol_check_reads_the_form", async () => {
    const form = makeForm({
      base_url: {
        "openai-completions": "https://api.example.com/v1",
        "anthropic-messages": "https://api.anthropic.com",
      },
    });
    const rules = providerFormRules({
      form,
      providers: [],
      currentSlug: "",
    });

    await expect(
      validate(rules.selectedProtocol.at(-1), undefined),
    ).rejects.toThrow("请选择投影使用的端点");
  });
});
