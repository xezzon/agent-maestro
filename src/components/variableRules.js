/**
 * 变量表（全局变量表与插件变量覆盖）的校验规则。
 *
 * 两处共用：名必填、同表内唯一、值非空白。差异各一条——全局表查名格式，
 * 插件覆盖查「名须已在全局变量表中声明」。共用部分只写一份，避免两套规则
 * 各改各的（issue #123）。
 * 唯一性读当前表单内的全部变量行，因此由接收 `form` 的工厂产出：
 * 调用方必须先拿到 `form` 再建规则。
 */

/** 变量名格式：仅允许字母、数字和下划线（ADR 0014）。 */
export const VARIABLE_NAME_PATTERN = /^[A-Za-z0-9_]+$/;

/** 变量名必填：两处共用。 */
export const VARIABLE_NAME_REQUIRED_RULE = {
  required: true,
  message: "请输入变量名",
};

/** 全局变量名的静态规则：必填与格式。 */
export const VARIABLE_NAME_RULES = [
  VARIABLE_NAME_REQUIRED_RULE,
  { pattern: VARIABLE_NAME_PATTERN, message: "变量名仅允许字母、数字和下划线" },
];

/**
 * 同表内唯一（大小写敏感）：重复的键整包写回时会静默合并，故提交前拦住。
 * @param {{ getFieldValue: (name: any) => any }} form
 * @param {(name: string) => string} duplicateMessage 重复时的提示（两处措辞不同）
 */
export function variableNameUniquenessRule(form, duplicateMessage) {
  return {
    validator: (_, value) => {
      if (!value) return Promise.resolve();
      const occurrences = (form.getFieldValue("variables") ?? []).filter(
        (variable) => variable?.name === value,
      ).length;
      return occurrences > 1
        ? Promise.reject(new Error(duplicateMessage(value)))
        : Promise.resolve();
    },
  };
}

/**
 * 插件覆盖：变量名必须已在全局变量表中声明（与后端 set_plugin_config 的校验一致）。
 * 用 `Object.hasOwn` 读自有属性，原型链上的名字不算声明过。
 * @param {import("@/api/variables").Variables} variables 全局变量表
 */
export function declaredVariableRule(variables) {
  return {
    validator: (_, value) => {
      if (!value) return Promise.resolve();
      return Object.hasOwn(variables, value)
        ? Promise.resolve()
        : Promise.reject(new Error(`变量「${value}」未在全局变量表中声明`));
    },
  };
}

/**
 * 变量值的规则：必填且不能为纯空白。
 * @param {string} label 值的称谓（「默认值」或「覆盖值」）
 */
export function variableValueRules(label) {
  return [
    { required: true, message: `请输入${label}` },
    { whitespace: true, message: `${label}不能为空白` },
  ];
}

/**
 * 全局变量表的规则组：名仅允许 `[A-Za-z0-9_]+`，同表内唯一。
 * @param {{ getFieldValue: (name: any) => any }} form
 */
export function globalVariableRules(form) {
  return {
    name: [
      ...VARIABLE_NAME_RULES,
      variableNameUniquenessRule(form, (name) => `变量名「${name}」重复`),
    ],
    value: variableValueRules("默认值"),
  };
}

/**
 * 插件变量覆盖的规则组：名须已在全局变量表声明，且同表内唯一。
 * 不重复查名格式——能通过声明检查的名字必然已在全局表中，格式已由那里保证。
 * @param {Object} deps
 * @param {{ getFieldValue: (name: any) => any }} deps.form
 * @param {import("@/api/variables").Variables} deps.variables 全局变量表
 */
export function overrideVariableRules({ form, variables }) {
  return {
    name: [
      VARIABLE_NAME_REQUIRED_RULE,
      declaredVariableRule(variables),
      variableNameUniquenessRule(form, (name) => `变量「${name}」重复`),
    ],
    value: variableValueRules("覆盖值"),
  };
}
