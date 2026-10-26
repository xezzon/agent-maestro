import { useState } from "react";
import { App } from "antd";

/**
 * 异步动作的共享样板：进行中状态 + 失败提示 + 收尾。
 *
 * `try / catch (err) { message.error(String(err)) } finally { setBusy(false) }`
 * 此前在十余处各写一遍——每处都要记得把 err 转成字符串、都要记得在 finally
 * 里收状态。漏一处就是「按钮一直转」或「错误不显示」。
 *
 * 失败只提示、不抛给调用方——调用方要区分成功失败时，改用自己的 try/catch。
 * @returns {[boolean, (action: () => Promise<void>) => Promise<void>]} 进行中状态与执行器
 */
export default function useAsyncAction() {
  const [busy, setBusy] = useState(false);
  // 静态 message 渲染在独立根里，拿不到 ConfigProvider 的动态主题；
  // useApp 这一份才跟着明暗走。
  const { message } = App.useApp();

  async function run(action) {
    setBusy(true);
    try {
      await action();
    } catch (err) {
      message.error(String(err));
    } finally {
      setBusy(false);
    }
  }

  return [busy, run];
}
