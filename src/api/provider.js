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
 * 后端 `list_providers` 返回的记录形态：`api_key` 恒为字符串——空串即未设置，
 * `secret://…` 即密钥链引用（旧配置里还可能是明文，等启动迁移换掉）。
 * 真值只进系统密钥链，配置只留引用（ADR 0002）；本视图改造前，界面原样
 * 携带这个字符串（回传它等价于「不变」，见第 5 票）。
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
 * 列表页/表单使用的 Provider 视图：原样携带 `api_key` 字符串（空串即未设置，
 * 引用 URI 即真值在密钥链里）。
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
 * 能力枚举串与界面标签。
 *
 * 标签此前在模型行（复选框分组）与只读表单（能力标签）各写一份——同一组
 * 文案两处维护，改一处漏一处就是「两处显示得不一样」。分组（能力 / 输入
 * 模态）只是排版，仍归模型行；标签本身归这里。
 * @type {Record<ModelCapability, string>}
 */
export const CAPABILITY_LABELS = {
  [CAPABILITY_TOOL_USE]: "工具调用",
  [CAPABILITY_IMAGE_IN]: "视觉",
  [CAPABILITY_THINKING]: "推理",
};

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
 * header 行的排序口径：按名，字节序（与记录层 `BTreeMap` 的键序一致）。
 * 读写两个方向共用这一条，故两边的顺序永远一致。
 * @param {CustomHeader} a
 * @param {CustomHeader} b
 * @returns {number}
 */
function compareHeaderRows(a, b) {
  return compareByteOrder(a.name, b.name);
}

/**
 * 名→值映射展开成行列表（读路径）。
 * @param {Record<string, string>|undefined} customHeader
 * @returns {CustomHeader[]}
 */
function toHeaderRows(customHeader) {
  return Object.entries(customHeader ?? {})
    .map(([name, value]) => ({ name, value }))
    .sort(compareHeaderRows);
}

/**
 * 行列表还原成名→值映射（写路径）。
 *
 * 按名排序后再还原：映射本无序，落盘产物得是确定的（issue #58 决定 21）。
 * 排序前先拷贝一份——行列表是调用方（表单）持有的数组，不能就地排序。
 * @param {CustomHeader[]|undefined} rows
 * @returns {Record<string, string>}
 */
function fromHeaderRows(rows) {
  return Object.fromEntries(
    (rows ?? [])
      .slice()
      .sort(compareHeaderRows)
      .map(({ name, value }) => [name, value]),
  );
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
 * 已配置的协议：端点只填空白视为未配置，与写入前的归一化同一口径。
 *
 * 「哪些端点算已配置」此前写了三遍（表单的整体校验、协议单选框的可用性、
 * 写入负载的过滤），三处口径一旦漂移，界面允许填的与实际落盘的就不一致。
 * 这里给出唯一一份口径（ADR 0016）。
 * @param {Partial<Endpoints>|undefined} baseUrl
 * @returns {ProviderProtocol[]}
 */
export function configuredProtocols(baseUrl) {
  return PROVIDER_PROTOCOLS.filter(
    (protocol) => (baseUrl?.[protocol] ?? "").trim() !== "",
  );
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
  const endpoints = normalizeEndpoints(provider.base_url);
  const base_url = Object.fromEntries(
    configuredProtocols(endpoints).map((protocol) => [
      protocol,
      endpoints[protocol],
    ]),
  );
  const selected_protocol =
    provider.selected_protocol && base_url[provider.selected_protocol]
      ? provider.selected_protocol
      : null;
  const custom_header = fromHeaderRows(provider.custom_header);
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
      custom_header: toHeaderRows(provider.custom_header),
      // 后端的「未设置」约定：未配置的凭证恒为空串（`SecretText` 类型的序列化
      // 形态，见 src-tauri/src/secret/text.rs）。这一句是那条约定的唯一前端落点。
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
