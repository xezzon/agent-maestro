import { useState } from "react";
import { Card, Flex, message, Switch, Tag, Typography } from "antd";
import { setProviderEnabled } from "../../api/provider";
import ProviderForm from "./ProviderForm";
import ProviderReadonlyForm from "./ProviderReadonlyForm";

/**
 * 更新态不做 slug 冲突校验（slug 不可编辑）。空数组提到模块级，
 * 让表单规则在重渲染之间保持同一引用、不被 useMemo 重建。
 */
const NO_SLUG_CONFLICTS = [];

/**
 * @param {Object} param0
 * @param {import("../../api/provider").Provider} param0.provider
 * @param {() => Promise<void>} param0.onReload
 */
export default function ProviderCard({ provider, onReload }) {
  const [editing, setEditing] = useState(false);
  // 开关与编辑表单互斥：任一方进行中另一方整体禁用。否则切换成功后、
  // 列表刷新落地前提交的表单仍持有切换前的 provider，会把刚完成的禁用改回启用。
  const [busy, setBusy] = useState(false);

  /** 开关独立于编辑表单：切换失败保持原状态。 */
  async function handleToggleEnabled(enabled) {
    setBusy(true);
    try {
      await setProviderEnabled(provider.slug, enabled);
      await onReload();
    } catch (err) {
      message.error(String(err));
    } finally {
      setBusy(false);
    }
  }

  const title = (
    <Flex justify="space-between" align="center">
      <Flex align="center" gap={8}>
        <Typography.Text strong>{provider.slug}</Typography.Text>
        {!provider.enabled && <Tag>已禁用</Tag>}
      </Flex>
      <Switch
        checked={provider.enabled}
        checkedChildren="启用"
        unCheckedChildren="禁用"
        disabled={busy}
        onChange={handleToggleEnabled}
      />
    </Flex>
  );

  return <Card title={title} className={provider.enabled ? undefined : "provider-disabled"}>
    {editing
      ? <ProviderForm
        provider={provider}
        providers={NO_SLUG_CONFLICTS}
        disabled={busy}
        onBusyChange={setBusy}
        onFinish={(refresh) => {
          setEditing(false)
          if (refresh) {
            onReload();
          }
        }}
      />
      : <ProviderReadonlyForm
        provider={provider}
        onEdit={() => setEditing(true)}
        afterDelete={onReload}
      />
    }
  </Card>;
}
