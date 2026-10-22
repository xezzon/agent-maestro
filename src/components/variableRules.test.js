import { describe, expect, it } from "vitest";
import {
  VARIABLE_NAME_RULES,
  globalVariableRules,
  overrideVariableRules,
} from "./variableRules.js";

/**
 * 构造 antd 表单依赖的最小替身：`getFieldValue("variables")` 读出当前表单内的变量行。
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

const EMPTY_FORM = makeForm({ variables: [] });
const GLOBAL_VARIABLES = { GATEWAY_ID: "gw-1", BASE_URL: "https://api.example.com" };

describe("变量名格式", () => {
  it("letters_digits_and_underscores_pass_the_format_rule", () => {
    const { pattern } = patternRule(VARIABLE_NAME_RULES);

    expect(
      ["A", "GATEWAY_ID", "gateway_id2", "_private", "123"].every((name) =>
        pattern.test(name),
      ),
      "字母、数字、下划线组成的名字合法（大小写敏感）",
    ).toBe(true);
  });

  it("names_outside_the_allowed_charset_fail_the_format_rule", () => {
    const { pattern } = patternRule(VARIABLE_NAME_RULES);

    expect(
      ["GATEWAY-ID", "GATEWAY ID", " gateway", "gateway.id", "变量", ""].filter(
        (name) => pattern.test(name),
      ),
      "连字符、空格、点、非 ASCII 字符与空串都不符合变量名格式",
    ).toStrictEqual([]);
  });

  it("variable_name_is_required", () => {
    expect(
      VARIABLE_NAME_RULES.some((rule) => rule.required === true),
      "变量名必填",
    ).toBe(true);
  });

  it("the_global_table_wires_the_format_rule", () => {
    const { pattern, message } = patternRule(
      globalVariableRules(EMPTY_FORM).name,
    );

    expect(pattern.test("BAD-NAME"), "全局变量表用格式规则拦住非法名").toBe(false);
    expect(message).toContain("仅允许字母、数字和下划线");
  });
});

describe("同表内变量名唯一", () => {
  it("a_duplicated_global_variable_name_is_rejected", async () => {
    const form = makeForm({
      variables: [{ name: "GATEWAY_ID" }, { name: "GATEWAY_ID" }],
    });

    await expect(
      validate(globalVariableRules(form).name.at(-1), "GATEWAY_ID"),
    ).rejects.toThrow("变量名「GATEWAY_ID」重复");
  });

  it("a_duplicated_override_name_is_rejected", async () => {
    const form = makeForm({
      variables: [{ name: "GATEWAY_ID" }, { name: "GATEWAY_ID" }],
    });
    const rules = overrideVariableRules({ form, variables: GLOBAL_VARIABLES });

    await expect(validate(rules.name.at(-1), "GATEWAY_ID")).rejects.toThrow(
      "变量「GATEWAY_ID」重复",
    );
  });

  it("a_unique_name_passes_in_both_tables", async () => {
    const form = makeForm({ variables: [{ name: "GATEWAY_ID" }] });

    await expect(
      validate(globalVariableRules(form).name.at(-1), "BASE_URL"),
    ).resolves.toBeUndefined();
    await expect(
      validate(
        overrideVariableRules({ form, variables: GLOBAL_VARIABLES }).name.at(
          -1,
        ),
        "BASE_URL",
      ),
    ).resolves.toBeUndefined();
  });

  it("the_uniqueness_check_is_case_sensitive", async () => {
    const form = makeForm({ variables: [{ name: "GATEWAY_ID" }] });

    await expect(
      validate(globalVariableRules(form).name.at(-1), "gateway_id"),
    ).resolves.toBeUndefined();
  });

  it("an_empty_name_is_left_to_the_required_rule", async () => {
    const form = makeForm({ variables: [{ name: "" }, { name: "" }] });

    await expect(
      validate(globalVariableRules(form).name.at(-1), ""),
    ).resolves.toBeUndefined();
  });
});

describe("插件覆盖的变量名须已在全局变量表中声明", () => {
  const form = makeForm({ variables: [] });

  it("an_undeclared_name_is_rejected", async () => {
    const rules = overrideVariableRules({ form, variables: GLOBAL_VARIABLES });

    await expect(
      validate(rules.name.at(-2), "NOT_DECLARED"),
    ).rejects.toThrow("变量「NOT_DECLARED」未在全局变量表中声明");
  });

  it("a_declared_name_passes", async () => {
    const rules = overrideVariableRules({ form, variables: GLOBAL_VARIABLES });

    await expect(
      validate(rules.name.at(-2), "GATEWAY_ID"),
    ).resolves.toBeUndefined();
  });

  it("an_empty_name_is_left_to_the_required_rule", async () => {
    const rules = overrideVariableRules({ form, variables: GLOBAL_VARIABLES });

    await expect(validate(rules.name.at(-2), "")).resolves.toBeUndefined();
  });

  it("inherited_object_properties_are_not_declared_variables", async () => {
    const rules = overrideVariableRules({ form, variables: GLOBAL_VARIABLES });

    await expect(validate(rules.name.at(-2), "toString")).rejects.toThrow(
      "未在全局变量表中声明",
    );
  });
});

describe("变量值不能为空白", () => {
  it("a_blank_global_default_is_rejected", () => {
    const valueRules = globalVariableRules(EMPTY_FORM).value;

    expect(valueRules).toContainEqual({ required: true, message: "请输入默认值" });
    expect(valueRules).toContainEqual({
      whitespace: true,
      message: "默认值不能为空白",
    });
  });

  it("a_blank_override_value_is_rejected", () => {
    const valueRules = overrideVariableRules({
      form: EMPTY_FORM,
      variables: GLOBAL_VARIABLES,
    }).value;

    expect(valueRules).toContainEqual({ required: true, message: "请输入覆盖值" });
    expect(valueRules).toContainEqual({
      whitespace: true,
      message: "覆盖值不能为空白",
    });
  });
});
