import { useState } from "react";
import { App as AntApp, Card, Flex, Radio, Typography } from "antd";
import { THEME_DARK, THEME_LIGHT, THEME_SYSTEM } from "@/api/preferences";
import { usePreferences } from "@/preferences/PreferencesProvider";

/** 主题三态：亮色 / 暗色 / 跟随系统。 */
const THEME_OPTIONS = [
  { value: THEME_LIGHT, label: "亮色" },
  { value: THEME_DARK, label: "暗色" },
  { value: THEME_SYSTEM, label: "跟随系统" },
];

/**
 * 偏好页：本期只有「外观」一组，提供主题三态。
 *
 * 选完即生效并写盘，没有保存按钮——少一步操作。写盘失败时偏好上下文回滚到切换
 * 前的值，原因在这里提示（提示落在 `ConfigProvider` 之内，故由界面层给出）。
 */
export default function PreferencesPage() {
  const { preferences, setTheme } = usePreferences();
  // 静态 `message` 拿不到 ConfigProvider 的上下文（暗色下弹窗仍是亮色），
  // 故用 `App.useApp()` 取可消费上下文的那一份。
  const { message } = AntApp.useApp();
  const [saving, setSaving] = useState(false);

  async function handleChange(theme) {
    setSaving(true);
    try {
      await setTheme(theme);
    } catch (err) {
      message.error(String(err));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="page">
      <div className="page-header">
        <Typography.Title level={4} style={{ margin: 0 }}>
          偏好
        </Typography.Title>
      </div>

      <Card title="外观">
        <Flex vertical gap={12}>
          <Flex align="center" gap={16}>
            <span>主题</span>
            <Radio.Group
              optionType="button"
              buttonStyle="solid"
              options={THEME_OPTIONS}
              value={preferences.theme}
              disabled={saving}
              onChange={(event) => handleChange(event.target.value)}
            />
          </Flex>
          <Typography.Text type="secondary">
            「跟随系统」表示应用外观随操作系统的明暗设置变化。
          </Typography.Text>
        </Flex>
      </Card>
    </div>
  );
}
