/**
 * 列表命令返回的 Provider：`api_key` 为密钥引用（`secret://` URI），
 * 未设置时键缺省；密钥真值永不出后端（ADR 0002）。
 * @typedef {Object} Provider
 * @property {Endpoints} base_url
 * @property {string=} api_key 密钥引用；未设置时缺省。
 * @property {Model[]} models 保序模型列表。
 */
/**
 * @typedef {record<string, string>} Endpoints
 */
/**
 * Provider 下的一条模型（界面形态）：模型 ID 不做字符集限制，
 * 同一 Provider 内唯一（大小写敏感）；显示名留空时界面回退显示 id。
 * @typedef {Object} Model
 * @property {string} id
 * @property {string} display_name 后端落盘为 `null`，界面形态为空串。
 */
/**
 * 创建/更新命令中 `api_key` 的负载形态（ADR 0002 三态契约）：
 * `value` 有值=覆盖写入，仅 `slug`=清除，`api_key` 缺省或 null=不变。
 * @typedef {Object} ApiKeyPayload
 * @property {string} slug
 * @property {string=} value
 */
/**
 * @typedef ProviderFormData
 * @property {string} slug
 * @property {string} protocol
 * @property {string} base_url
 * @property {Model[]} models 保序模型列表。
 * @property {ApiKeyPayload=} api_key 设置/更换/清除密钥的表单交互尚未实现。
 */
import { invoke } from "@tauri-apps/api/core";

/**
 * 落盘形态：显示名留空（含空白）即存 `null`，避免空串与 null 两种「无显示名」形态；
 * 模型 ID 原样保存，数组保序。
 * @param {Model[]=} models
 * @returns {Array<{id: string, display_name: string | null}>}
 */
function toModelsPayload(models) {
  return (models ?? []).map((model) => ({
    id: model.id,
    display_name: model.display_name?.trim() || null,
  }));
}

/**
 * 创建/更新命令共用的 `provider` 负载：`base_url` 按所选协议落对应槽位（ADR 0003），
 * 模型列表整包替换（保序）。
 * @param {ProviderFormData} provider
 */
function toProviderPayload(provider) {
  return {
    ...provider,
    base_url: {
      [provider.protocol]: provider.base_url,
    },
    models: toModelsPayload(provider.models),
  };
}

/**
 * @param {ProviderFormData} provider
 */
export async function createProvider(provider) {
  await invoke("create_provider", {
    slug: provider.slug,
    provider: toProviderPayload(provider),
  });
}

/**
 * @param {ProviderFormData} provider
 */
export async function updateProvider(provider) {
  await invoke("update_provider", {
    slug: provider.slug,
    provider: toProviderPayload(provider),
  });
}

/**
 * @returns {Promise<ProviderFormData[]>}
 */
export async function listProviders() {
  /**
   * @type {record<string, Provider>}
   */
  const providers = await invoke("list_providers");
  return Object.entries(providers)
    .map(([slug, provider]) => ({
      ...provider,
      slug,
      protocol: Object.keys(provider.base_url)[0],
      base_url: Object.values(provider.base_url)[0],
      api_key_set: !!provider.api_key,
    }))
    .sort((a, b) => a.slug.localeCompare(b.slug));
}

/**
 * @param {string} slug
 * @returns {Promise<string[]>} warnings
 */
export async function deleteProvider(slug) {
  return await invoke("delete_provider", { slug });
}
