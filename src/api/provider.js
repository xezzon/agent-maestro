/**
 * @typedef {Object} Provider
 * @property {Endpoints} base_url
 * @property {StringOrSecret} api_key
 */
/**
 * @typedef {record<string, string>} Endpoints
 */
/**
 * @typedef {string} StringOrSecret
 */
/**
 * @typedef ProviderFormData
 * @property {string} slug
 * @property {string} protocol
 * @property {string} base_url
 * @property {string?} api_key
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
