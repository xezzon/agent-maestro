/**
 * @typedef {'openai-completions' | 'anthropic-messages'} ProviderProtocol
 */
/**
 * 两个协议的端点槽位恒存在，空串表示该协议未配置（后端落盘时省略空槽位的键）。
 * @typedef {Record<ProviderProtocol, string>} Endpoints
 */
/**
 * 模型能力的枚举串，落盘顺序即此声明序（`tool_use` → `image_in` → `thinking`）。
 * 界面按「能力 / 输入模态」分组展示，分组不改变落盘形状（「视觉」即 `image_in`）。
 * @typedef {'tool_use' | 'image_in' | 'thinking'} ModelCapability
 */
/**
 * 模型的 token 能力上限（声明式，Maestro 不据此裁剪请求）。
 * 未设置的子项缺键即未设置，不填任何默认值。
 * @typedef {Object} ModelLimit
 * @property {number=} context_window 上下文窗口。
 * @property {number=} max_input 单次最大输入。
 * @property {number=} max_output 最大输出。
 */
/**
 * Provider 下的一条模型（界面形态）：模型 ID 不做字符集限制，
 * 同一 Provider 内唯一（大小写敏感）；显示名留空时界面回退显示 id。
 * @typedef {Object} Model
 * @property {string} id
 * @property {string} display_name 后端落盘为 `null`，界面形态为空串。
 * @property {ModelLimit} limit 恒为对象，界面归一为「未设置即缺键」。
 * @property {ModelCapability[]} capabilities 集合语义，界面归一为枚举序去重。
 */
/**
 * Provider 的一条自定义 HTTP header（界面形态）：一行一条，名与值都必填。
 * 名的非空、HTTP token 字符集与忽略大小写的唯一性都只在界面内校验
 * （记录层存大小写敏感的原名，见 issue #58 决定 11）；值是凭证，
 * 只读卡片只列名、不显示值。
 * @typedef {Object} CustomHeader
 * @property {string} name
 * @property {string} value
 */
/**
 * 后端 `list_providers` 返回的记录形态：`api_key` 为明文凭证，
 * 空串即未设置；第一期凭证随配置文件落盘，不做密钥链（ADR 0002 推迟采纳）。
 * `base_url` 只包含已配置的槽位，缺键即未配置。
 * `custom_header` 是名→值映射（无序），缺键即空映射。
 * 创建/更新命令的 `provider` 负载复用同一组字段名，但恒带 `slug`、
 * `selected_protocol`（未选为 `null`）与 `api_key`（空串即未设置）；
 * 写入字段集合固定，见 `toProviderPayload`。
 * @typedef {Object} ProviderRequest
 * @property {string} slug
 * @property {boolean} enabled 禁用后不参与投影；旧配置缺此字段视为启用。
 * @property {Partial<Endpoints>} base_url
 * @property {ProviderProtocol|null} selected_protocol 界面所选端点；未选为 `null`。
 * @property {string=} api_key
 * @property {Record<string, string>=} custom_header 值是明文凭证。
 * @property {Model[]} models 保序模型列表。
 */
/**
 * 列表页/表单使用的 Provider 视图：原样携带明文 `api_key`（空串即未设置）。
 * 创建/更新时按写入字段集回传（见 `toProviderPayload`），只读字段不回传。
 * @typedef {Object} Provider
 * @property {string} slug
 * @property {boolean} enabled 禁用后不参与投影；禁用态仍可编辑。
 * @property {Endpoints} base_url
 * @property {ProviderProtocol|null} selected_protocol 投影使用的协议；未选为 `null`。
 * @property {Model[]} models
 * @property {CustomHeader[]} custom_header 界面形态的行列表；读入时按名排序展开。
 * @property {string=} api_key
 * @property {boolean} api_key_set 只读：是否已配置凭证（读取路径生成，不随写入负载回传）。
 */
import { invoke } from "@tauri-apps/api/core";

/** @type {ProviderProtocol} */
export const OPENAI_COMPLETIONS = "openai-completions";
/** @type {ProviderProtocol} */
export const ANTHROPIC_MESSAGES = "anthropic-messages";
/** @type {ProviderProtocol[]} */
export const PROVIDER_PROTOCOLS = [OPENAI_COMPLETIONS, ANTHROPIC_MESSAGES];
/** @type {ModelCapability} 工具调用。 */
export const CAPABILITY_TOOL_USE = "tool_use";
/** @type {ModelCapability} 视觉（输入模态）。 */
export const CAPABILITY_IMAGE_IN = "image_in";
/** @type {ModelCapability} 推理。 */
export const CAPABILITY_THINKING = "thinking";
/** @type {ModelCapability[]} 能力枚举的声明序（`normalizeModel` 按此序排序去重）。 */
const MODEL_CAPABILITIES = [
  CAPABILITY_TOOL_USE,
  CAPABILITY_IMAGE_IN,
  CAPABILITY_THINKING,
];

/**
 * 按字节序比较两个字符串（而非 `localeCompare`——那是随语言环境变化的排序）：
 * 与记录层 `BTreeMap<String, _>` 的键序一致，slug 列表与 header 行共用这一套口径。
 * 比较的是 UTF-16 码元序，与 Rust 的 UTF-8 字节序只在 BMP 内等价：slug
 * （`[a-z][a-z0-9_-]*`）与 header 名（HTTP token）都是 ASCII，实践中不分歧。
 * @param {string} a
 * @param {string} b
 * @returns {number}
 */
function compareByteOrder(a, b) {
  return a < b ? -1 : a > b ? 1 : 0;
}

/**
 * 把后端可能缺键的 `base_url` 补齐为两个槽位恒存在的形态，缺键补空串。
 * @param {Partial<Endpoints>|undefined} baseUrl
 * @returns {Endpoints}
 */
export function normalizeEndpoints(baseUrl) {
  return Object.fromEntries(
    PROVIDER_PROTOCOLS.map((protocol) => [protocol, baseUrl?.[protocol] ?? ""]),
  );
}

/**
 * 归一化一条模型：`limit` 恒为对象且「空即缺键」（InputNumber 清空得到 `null`），
 * `capabilities` 按枚举声明序排序去重（取值受枚举约束，界面不求并集）。
 * 导出供单元测试直接调用（调用方是 `listProviders` / `toProviderPayload`）。
 * @param {Model} model
 * @returns {Model}
 */
export function normalizeModel(model) {
  const limit = Object.fromEntries(
    Object.entries(model.limit ?? {}).filter(([, value]) => value != null),
  );
  const selected = new Set(model.capabilities ?? []);
  return {
    ...model,
    limit,
    capabilities: MODEL_CAPABILITIES.filter((capability) =>
      selected.has(capability),
    ),
  };
}

/**
 * 创建/更新命令共用的 `provider` 负载：字段集合固定为写入字段
 * （slug、enabled、base_url、selected_protocol、api_key、custom_header、models），
 * 只读视图字段（如 `api_key_set`）不进负载——后端 `ProviderRequest` 不拒绝未知字段，
 * 视图模型越过持久化边界只会被静默丢弃。
 *
 * `base_url` 只携带已填写的槽位，`selected_protocol` 归一化为已填槽位之一
 * （否则 `null`，投影时由插件兜底）。`api_key` 与模型列表随整包替换回传（保序）；
 * `custom_header` 由行列表还原为名→值映射。归一化归前端：映射本无序，
 * 按名排序后落盘才能让产物确定（issue #58 决定 21）。
 * 导出供单元测试直接调用（调用方是 `createProvider` / `updateProvider`）。
 * @param {Provider} provider
 * @returns {ProviderRequest}
 */
export function toProviderPayload(provider) {
  const base_url = Object.fromEntries(
    Object.entries(normalizeEndpoints(provider.base_url)).filter(
      ([, url]) => url.trim() !== "",
    ),
  );
  const selected_protocol =
    provider.selected_protocol && base_url[provider.selected_protocol]
      ? provider.selected_protocol
      : null;
  // 按字节序（compareByteOrder）与记录层 BTreeMap 的排序一致。
  const custom_header = Object.fromEntries(
    (provider.custom_header ?? [])
      .map(({ name, value }) => [name, value])
      .sort(([a], [b]) => compareByteOrder(a, b)),
  );
  return {
    slug: provider.slug,
    enabled: provider.enabled,
    base_url,
    selected_protocol,
    api_key: provider.api_key ?? "",
    custom_header,
    models: (provider.models ?? []).map(normalizeModel),
  };
}

/**
 * @param {Provider} provider
 */
export async function createProvider(provider) {
  await invoke("create_provider", {
    provider: toProviderPayload(provider),
  });
}

/**
 * @param {Provider} provider
 */
export async function updateProvider(provider) {
  await invoke("update_provider", {
    provider: toProviderPayload(provider),
  });
}

/**
 * 读取 Provider 列表：端点槽位补空串，模型归一化（见 `normalizeModel`），
 * slug 列表与 header 行都按字节序排序（与记录层 `BTreeMap` 的键序一致）。
 * @returns {Promise<Provider[]>}
 */
export async function listProviders() {
  /**
   * @type {Record<string, ProviderRequest>}
   */
  const providers = await invoke("list_providers");
  return Object.entries(providers)
    .map(([slug, provider]) => ({
      ...provider,
      slug,
      base_url: normalizeEndpoints(provider.base_url),
      selected_protocol: provider.selected_protocol ?? null,
      // 记录层是 BTreeMap，但 JSON 传进 JS 对象后键序不保证（整数字样键还会前置），
      // 故显式按名排序展开成行列表，与表单的 Form.List 对齐。
      custom_header: Object.entries(provider.custom_header ?? {})
        .map(([name, value]) => ({ name, value }))
        .sort((a, b) => compareByteOrder(a.name, b.name)),
      api_key_set: !!provider.api_key,
      models: (provider.models ?? []).map(normalizeModel),
    }))
    .sort((a, b) => compareByteOrder(a.slug, b.slug));
}

/**
 * 启用/禁用 Provider：禁用后不再投影到各工具。切换只改状态，不触发插值校验。
 * @param {string} slug
 * @param {boolean} enabled
 */
export async function setProviderEnabled(slug, enabled) {
  await invoke("set_provider_enabled", { slug, enabled });
}

/**
 * Provider 的端点、模型与 API Key 随记录一并删除。
 * @param {string} slug
 */
export async function deleteProvider(slug) {
  await invoke("delete_provider", { slug });
}
