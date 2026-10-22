/**
 * URL 校验规则：协议白名单 + 各自的文案。
 *
 * Base URL（http/https）与插件来源（仅 https）此前各写一遍同形的
 * `new URL` + 协议白名单，规则一改要改两处——漏一处就是「这个框能填、
 * 那个框不能填」的差别没有理由。规则本身只有一份，白名单与文案由调用方给。
 */

/**
 * 产出一条 URL 校验规则。
 * @param {Object} options
 * @param {string[]} options.schemes 允许的协议（含尾冒号，如 `["https:"]`）
 * @param {string} options.invalidUrl 不是合法 URL 时的文案
 * @param {string} options.badScheme 协议不在白名单里时的文案
 * @returns {{ validator: (rule: any, value: any) => Promise<void> }}
 */
export function urlRule({ schemes, invalidUrl, badScheme }) {
  return {
    validator: (_, value) => {
      if (!value) return Promise.resolve();
      let url;
      try {
        url = new URL(value);
      } catch {
        return Promise.reject(new Error(invalidUrl));
      }
      if (!schemes.includes(url.protocol)) {
        return Promise.reject(new Error(badScheme));
      }
      return Promise.resolve();
    },
  };
}
