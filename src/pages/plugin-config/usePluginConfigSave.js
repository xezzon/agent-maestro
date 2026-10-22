import { message } from "antd";
import { setPluginConfig } from "../../api/plugins";
import useAsyncAction from "../../hooks/useAsyncAction";

/**
 * 插件配置保存的共享样板：saving 状态 + 整包写回 + 结果 message。
 * 返回 [saving, save]；save(config, patch, onSaved) 把 patch 并入 config
 * 整包写回该插件的配置（ADR 0018），成功后调用 onSaved（由调用方决定是否刷新）。
 * 保存成功后页面未必有可见变化（表单段刷新后与提交前同形），故成功也提示。
 * @param {string} source 插件来源（条目身份，同 PluginEntry::source）
 * @returns {[boolean, (config: import("../../api/plugins").PluginConfig, patch: Object, onSaved: () => void) => Promise<void>]}
 */
export default function usePluginConfigSave(source) {
  const [saving, run] = useAsyncAction();
  async function save(config, patch, onSaved) {
    await run(async () => {
      await setPluginConfig(source, { ...config, ...patch });
      message.success("插件配置已保存");
      onSaved();
    });
  }
  return [saving, save];
}
