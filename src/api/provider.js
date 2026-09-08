/**
 * @typedef {'openai-completions' | 'anthropic-messages'} ProviderProtocol
 */
/**
 * @typedef {Record<ProviderProtocol, string>} Endpoints
 */
/**
 * Provider 下的一条模型（界面形态）：模型 ID 不做字符集限制，
 * 同一 Provider 内唯一（大小写敏感）；显示名留空时界面回退显示 id。
 * @typedef {Object} Model
 * @property {string} id
 * @property {string} display_name 后端落盘为 `null`，界面形态为空串。
 */
/**
 * 后端 `list_providers` 返回的记录形态：`api_key` 为明文凭证，
 * 空串即未设置；第一期凭证随配置文件落盘，不做密钥链（ADR 0002 推迟采纳）。
 * @typedef {Object} ProviderRequest
 * @property {string} slug
 * @property {Endpoints} base_url
 * @property {string=} api_key
 * @property {Model[]} models 保序模型列表。
 */
/**
 * 列表页/表单使用的 Provider 视图：原样携带明文 `api_key`（空串即未设置），
 * 创建/更新时整包回传。
 * @typedef {Object} Provider
 * @property {string} slug
 * @property {ProviderProtocol} protocol
 * @property {string} base_url
 * @property {Model[]} models
 * @property {string=} api_key
 */
import { invoke } from "@tauri-apps/api/core";

/**
 * 创建/更新命令共用的 `provider` 负载：`base_url` 按所选协议落对应槽位（ADR 0003），
 * `api_key` 与模型列表随整包替换回传（保序）。
 * @param {Provider} provider
 */
function toProviderPayload(provider) {
  return {
    ...provider,
    base_url: {
      [provider.protocol]: provider.base_url,
    },
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
      protocol: Object.keys(provider.base_url)[0],
      base_url: Object.values(provider.base_url)[0],
      api_key_set: !!provider.api_key,
    }))
    .sort((a, b) => a.slug.localeCompare(b.slug));
}

/**
 * Provider 的端点、模型与 API Key 随记录一并删除。
 * @param {string} slug
 */
export async function deleteProvider(slug) {
  await invoke("delete_provider", { slug });
}
