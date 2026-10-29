import { Button, Flex, Form, Input, Tag } from "antd";
import { apiKeyStatePresentation } from "./apiKeyState";
import PlaintextPendingAlert from "./PlaintextPendingAlert";

/**
 * API Key 的编辑区：单向写入的界面形态。
 *
 * 真值只往系统密钥链写，界面永不回显（ADR 0002）：未设置时直接给输入框，留空
 * 即不设置；已设置 / 明文态只给「更换 API Key」按钮，展开后留空即清除。
 *
 * 展开与否是组件本地状态（与 `enabled` 同类，不是 `Form.Item`）——它决定这次
 * 保存对密钥做什么（见 `resolveApiKeyAction`），故密钥值从不进表单状态。
 *
 * @param {Object} param0
 * @param {import("@/api/provider").ApiKeyState} param0.state 当前凭证状态
 * @param {import("@/api/provider").ApiKeyDraft} param0.draft 表单草稿（`null` = 未展开）
 * @param {(draft: import("@/api/provider").ApiKeyDraft) => void} param0.onDraftChange
 */
export default function ApiKeyField({ state, draft, onDraftChange }) {
  const presentation = apiKeyStatePresentation(state);
  const expanded = draft !== null;

  return (
    <Form.Item
      label="API Key"
      extra="真值只写入系统密钥链，配置文件里只留引用。"
    >
      <Flex vertical gap={8}>
        {presentation?.pendingMigration ? <PlaintextPendingAlert /> : null}
        {expanded ? (
          <>
            <Input.Password
              value={draft.value}
              placeholder={presentation?.placeholder}
              autoComplete="new-password"
              visibilityToggle={false}
              onChange={(event) => onDraftChange({ value: event.target.value })}
            />
            {presentation?.hasCredential ? (
              <Button onClick={() => onDraftChange(null)}>取消更换</Button>
            ) : null}
          </>
        ) : (
          <Flex align="center" gap={8}>
            {presentation ? <Tag>{presentation.label}</Tag> : null}
            <Button onClick={() => onDraftChange({ value: "" })}>
              更换 API Key
            </Button>
          </Flex>
        )}
      </Flex>
    </Form.Item>
  );
}
