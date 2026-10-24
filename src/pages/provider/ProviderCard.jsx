import { useState } from "react";
import { Card, Flex, Switch, Tag, Typography } from "antd";
import { setProviderEnabled } from "@/api/provider";
import useAsyncAction from "@/hooks/useAsyncAction";
import ProviderForm from "./ProviderForm";
import ProviderReadonlyForm from "./ProviderReadonlyForm";
import { MODE_UPDATE } from "./formMode";

/**
 * @param {Object} param0
 * @param {import("@/api/provider").Provider} param0.provider
 * @param {() => Promise<void>} param0.onReload
 */
export default function ProviderCard({ provider, onReload }) {
  const [editing, setEditing] = useState(false);
  // 开关与编辑表单互斥：任一方进行中另一方整体禁用。否则切换成功后、
  // 列表刷新落地前提交的表单仍持有切换前的 provider，会把刚完成的禁用改回启用。
  // 真正的不变式是「表单打开期间不允许切换启用状态」——故编辑期间开关直接
  // 禁用，父组件不必知道表单内部的保存态。
  const [toggling, runToggle] = useAsyncAction();

  /** 开关独立于编辑表单：切换失败保持原状态。 */
  async function handleToggleEnabled(enabled) {
    await runToggle(async () => {
      await setProviderEnabled(provider.slug, enabled);
      await onReload();
    });
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
        disabled={toggling || editing}
        onChange={handleToggleEnabled}
      />
    </Flex>
  );

  return (
    <Card
      title={title}
      className={provider.enabled ? undefined : "provider-disabled"}
    >
      {editing ? (
        <ProviderForm
          mode={MODE_UPDATE}
          provider={provider}
          disabled={toggling}
          onCancel={() => setEditing(false)}
          onSaved={() => {
            setEditing(false);
            onReload();
          }}
        />
      ) : (
        <ProviderReadonlyForm
          provider={provider}
          onEdit={() => setEditing(true)}
          afterDelete={onReload}
        />
      )}
    </Card>
  );
}
