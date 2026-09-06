/**
 * 列表命令返回的 Provider：`api_key` 为密钥引用（`secret://` URI），
 * 未设置时键缺省；密钥真值永不出后端（ADR 0002）。
 * @typedef {Object} Provider
 * @property {Endpoints} base_url
 * @property {string=} api_key 密钥引用；未设置时缺省。
 */
/**
 * @typedef {record<string, string>} Endpoints
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
 * @property {ApiKeyPayload=} api_key 设置/更换/清除密钥的表单交互尚未实现。
 */
import { invoke } from "@tauri-apps/api/core";

/**
 * @param {ProviderFormData} provider
 */
export async function createProvider(provider) {
  await invoke("create_provider", {
    slug: provider.slug,
    provider: {
      ...provider,
      base_url: {
        [provider.protocol]: provider.base_url,
      },
    },
  });
}

/**
 * @param {ProviderFormData} provider
 */
export async function updateProvider(provider) {
  await invoke("update_provider", {
    slug: provider.slug,
    provider: {
      ...provider,
      base_url: {
        [provider.protocol]: provider.base_url,
      },
    },
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
