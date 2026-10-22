import {
  ANTHROPIC_MESSAGES,
  OPENAI_COMPLETIONS,
  PROVIDER_PROTOCOLS,
} from "../../api/provider";

/**
 * Provider 表单的校验规则。
 *
 * 不依赖表单实时值的规则是模块级常量：渲染时只组装数组，规则对象本身不重建。
 * 需要读整张表单的校验（跨字段、唯一性）由接收 `form`（或 `providers`）的
 * 工厂产出——依赖在创建规则时注入，调用方必须先声明 `form` 再建规则，
 * 不再依赖闭包的延迟求值。
 */

/** slug 格式：小写字母开头，只含小写字母、数字、连字符、下划线。 */
export const SLUG_PATTERN = /^[a-z][a-z0-9_-]*$/;

/** HTTP token 字符集（RFC 9110 的 tchar）：header 名必须是合法 token，工具侧才能解析。 */
export const HEADER_NAME_PATTERN = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;

/** slug 的静态规则：必填与格式。 */
export const SLUG_RULES = [
  { required: true, message: "请输入 slug" },
  {
    pattern: SLUG_PATTERN,
    message: "slug 需以小写字母开头，仅允许小写字母、数字、连字符和下划线",
  },
];

/**
 * slug 唯一性：与已有 Provider 冲突时报错。
 * 更新态传 `providers: []`（slug 不可编辑），因此不做冲突校验；即便如此，
 * 传入的 `currentSlug` 也保证把自己排除在外。
 * @param {Object} deps
 * @param {import("../../api/provider").Provider[]} deps.providers 当前存在的 Provider
 * @param {string} deps.currentSlug 正在编辑的 Provider 的 slug（新建时为空串）
 */
export function slugUniquenessRule({ providers, currentSlug }) {
  return {
    validator: (_, value) =>
      value !== currentSlug && providers.some((p) => p.slug === value)
        ? Promise.reject(new Error(`slug ${value} 已被使用`))
        : Promise.resolve(),
  };
}

/**
 * 端点本身：留空放行；填了就必须是 http(s) URL。
 * 两个协议槽位共用同一组规则。
 */
export const BASE_URL_RULES = [
  {
    validator: (_, value) => {
      if (!value) return Promise.resolve();
      let url;
      try {
        url = new URL(value);
      } catch {
        return Promise.reject(new Error("Base URL 不是合法的 URL"));
      }
      if (url.protocol !== "http:" && url.protocol !== "https:") {
        return Promise.reject(new Error("Base URL 仅支持 http(s) 地址"));
      }
      return Promise.resolve();
    },
  },
];

/**
 * 已配置的协议：端点只填空白视为未配置，与写入前的归一化同一口径。
 * @param {{ getFieldValue: (name: any) => any }} form
 * @returns {import("../../api/provider").ProviderProtocol[]}
 */
function filledProtocols(form) {
  return PROVIDER_PROTOCOLS.filter(
    (protocol) =>
      (form.getFieldValue(["base_url", protocol]) ?? "").trim() !== "",
  );
}

/**
 * 两个协议至少填一个端点：两个 URL 字段各自的 BASE_URL_RULES 都放行空值，
 * 这条整体校验挂在 openai 槽位上（提交时全量 validateFields 会一并触发）。
 * @param {{ getFieldValue: (name: any) => any }} form
 */
export function atLeastOneEndpointRule(form) {
  return {
    validator: () => {
      return filledProtocols(form).length > 0
        ? Promise.resolve()
        : Promise.reject(new Error("请至少填写一个协议的 Base URL"));
    },
  };
}

/**
 * 两个协议都配置时必须选定投影使用的协议；只配置一个时无需选择。
 * @param {{ getFieldValue: (name: any) => any }} form
 */
export function selectedProtocolRule(form) {
  return {
    validator: (_, value) => {
      const filled = filledProtocols(form);
      if (filled.length < 2) return Promise.resolve();
      return filled.includes(value)
        ? Promise.resolve()
        : Promise.reject(new Error("请选择投影使用的端点"));
    },
  };
}

/**
 * header 名的静态规则：非空、限 HTTP token 字符集。
 * 后端存大小写敏感的原名（issue #58 决定 11），这两条校验只在前端内联，
 * 不在 Rust 侧重复。
 */
export const HEADER_NAME_RULES = [
  { required: true, message: "请输入 Header 名" },
  {
    pattern: HEADER_NAME_PATTERN,
    message: "Header 名仅允许字母、数字与 !#$%&'*+-.^_`|~",
  },
];

/**
 * header 名在同一 Provider 内忽略大小写唯一：唯一性读的是当前表单内全部 header 行。
 * @param {{ getFieldValue: (name: any) => any }} form
 */
export function headerNameUniquenessRule(form) {
  return {
    validator: (_, value) => {
      if (!value) return Promise.resolve();
      const occurrences = (form.getFieldValue("custom_header") ?? []).filter(
        (header) => header?.name?.toLowerCase() === value.toLowerCase(),
      ).length;
      return occurrences > 1
        ? Promise.reject(
          new Error(`Header 名「${value}」在当前 Provider 内重复（忽略大小写）`),
        )
        : Promise.resolve();
    },
  };
}

/** header 值可能含凭证，必填；界面用普通输入框明文编辑。 */
export const HEADER_VALUE_RULES = [{ required: true, message: "请输入 Header 值" }];

/**
 * 模型 ID 非空（空白串视为空）且同一 Provider 内唯一（大小写敏感）。
 * 唯一性读的是当前表单内全部模型行。
 * @param {{ getFieldValue: (name: any) => any }} form
 */
export function modelIdRule(form) {
  return {
    validator: (_, value) => {
      const id = value ?? "";
      if (!id.trim()) {
        return Promise.reject(new Error("请输入模型 ID"));
      }
      const occurrences = (form.getFieldValue("models") ?? []).filter(
        (model) => model?.id === value,
      ).length;
      return occurrences > 1
        ? Promise.reject(new Error(`模型 ID「${value}」在当前 Provider 内重复`))
        : Promise.resolve();
    },
  };
}

/**
 * Provider 表单各字段的规则组：按字段名组装，供 `Form.Item` 直接取用。
 * 依赖在创建时注入（先声明 `form`，再调用本工厂）；不依赖表单的规则组
 * 直接复用模块级常量，调用方再把结果放进 `useMemo` 即可做到不随渲染重建。
 * @param {Object} deps
 * @param {{ getFieldValue: (name: any) => any }} deps.form
 * @param {import("../../api/provider").Provider[]} deps.providers
 * @param {string} deps.currentSlug 正在编辑的 Provider 的 slug（新建时为空串）
 */
export function providerFormRules({ form, providers, currentSlug }) {
  return {
    slug: [...SLUG_RULES, slugUniquenessRule({ providers, currentSlug })],
    // 按协议槽位分组，字段名复用 PROVIDER_PROTOCOLS 的常量，不另写一份键名。
    baseUrl: {
      [OPENAI_COMPLETIONS]: [...BASE_URL_RULES, atLeastOneEndpointRule(form)],
      [ANTHROPIC_MESSAGES]: BASE_URL_RULES,
    },
    selectedProtocol: [selectedProtocolRule(form)],
    headerName: [...HEADER_NAME_RULES, headerNameUniquenessRule(form)],
    headerValue: HEADER_VALUE_RULES,
    modelId: [modelIdRule(form)],
  };
}
