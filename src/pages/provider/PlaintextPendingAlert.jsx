import { Alert } from "antd";

/**
 * 「明文待迁移」告警：真值仍在配置文件里（启动迁移未成功，下次启动重试）。
 *
 * 只读卡片与编辑表单共用这一份 JSX 与文案——两处各写一遍的改法会漂移。
 */
export default function PlaintextPendingAlert() {
  return (
    <Alert
      type="warning"
      showIcon
      title="明文待迁移"
      description="API Key 仍以明文留在配置文件里。重启 Maestro 会重试迁移，也可以现在重新设置 API Key。"
    />
  );
}
