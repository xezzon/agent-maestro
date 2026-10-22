import { useState } from "react";
import { message } from "antd";

/**
 * 异步动作的共享样板：进行中状态 + 失败提示 + 收尾。
 *
 * `try / catch (err) { message.error(String(err)) } finally { setBusy(false) }`
 * 此前在十余处各写一遍——每处都要记得把 err 转成字符串、都要记得在 finally
 * 里收状态。漏一处就是「按钮一直转」或「错误不显示」。
 *
 * 失败默认只提示（不抛给调用方）；需要额外处理时传 `onError`（此时不再
 * 弹提示，由调用方决定怎么呈现）。
 * @param {Object} options
 * @param {(err: unknown) => void=} options.onError 失败时的额外处理
 * @returns {[boolean, (action: () => Promise<void>) => Promise<void>]} 进行中状态与执行器
 */
export default function useAsyncAction({ onError } = {}) {
  const [busy, setBusy] = useState(false);

  async function run(action) {
    setBusy(true);
    try {
      await action();
    } catch (err) {
      if (onError) onError(err);
      else message.error(String(err));
    } finally {
      setBusy(false);
    }
  }

  return [busy, run];
}
