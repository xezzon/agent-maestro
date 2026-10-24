/**
 * 最新请求闸门：给每次异步请求发一个序号，只有序号最大的那次才允许写回状态。
 *
 * 插件列表有两条并发的刷新路径：切页时的导航刷新与 Plugins 页的显式刷新。
 * 较慢的旧请求可能在较新的请求之后落地，把插件列表覆盖成旧快照——导航项、
 * Provider 页的投影按钮与插件卡片于是显示过期状态，直到下一次刷新。
 *
 * 闸门把「这次结果还该不该写回」变成一句纯判断，因而可以单独测；组件本身
 * 不必再各自记一份序号。
 */

/**
 * 建一个最新请求闸门。
 * @returns {{ begin: () => number, isLatest: (id: number) => boolean }}
 */
export function createLatestRequestGate() {
  let latest = 0;
  return {
    /** 登记一次新请求，返回它的序号。 */
    begin() {
      latest += 1;
      return latest;
    },
    /**
     * 该序号是否仍是最新的一次请求。
     * 不是则结果已过期，调用方不得据此写回状态。
     * @param {number} id `begin` 返回的序号
     * @returns {boolean}
     */
    isLatest(id) {
      return id === latest;
    },
  };
}
