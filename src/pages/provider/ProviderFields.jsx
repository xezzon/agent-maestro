import { Form, Input, Radio } from "antd";
import {
  ANTHROPIC_MESSAGES,
  OPENAI_COMPLETIONS,
  configuredProtocols,
} from "../../api/provider";
import { MODE_UPDATE } from "./formMode";

/** 投影协议的可选项：标签带协议名，界面上一眼对得上落盘值。 */
const PROTOCOL_OPTIONS = [
  {
    value: OPENAI_COMPLETIONS,
    label: "openai-completions（OpenAI 兼容 Chat Completions）",
  },
  {
    value: ANTHROPIC_MESSAGES,
    label: "anthropic-messages（Anthropic Messages API）",
  },
];

/**
 * Provider 自身的标量字段：slug、两个协议槽位的端点、投影使用的协议与 API Key。
 * 行列表（自定义 Header、模型）分别在 `HeaderRows` / `ModelRows` 里。
 *
 * 端点槽位实时决定投影选项的可用性：没填端点的协议不可选。
 *
 * @param {Object} param0
 * @param {"create" | "update"} param0.mode 更新态 slug 只读
 * @param {{ slug: Object[], baseUrl: Record<string, Object[]>, selectedProtocol: Object[] }} param0.rules
 */
export default function ProviderFields({ mode, rules }) {
  const endpoints = Form.useWatch("base_url") ?? {};
  const filledProtocols = configuredProtocols(endpoints);

  return (
    <>
      <Form.Item
        name="slug"
        label="Slug"
        extra="slug 创建后不可修改"
        rules={rules.slug}
      >
        <Input disabled={mode === MODE_UPDATE} />
      </Form.Item>

      <Form.Item
        name={["base_url", OPENAI_COMPLETIONS]}
        label={`Base URL（${OPENAI_COMPLETIONS}）`}
        rules={rules.baseUrl[OPENAI_COMPLETIONS]}
      >
        <Input placeholder="例如 http://localhost:11434/v1" />
      </Form.Item>

      <Form.Item
        name={["base_url", ANTHROPIC_MESSAGES]}
        label={`Base URL（${ANTHROPIC_MESSAGES}）`}
        rules={rules.baseUrl[ANTHROPIC_MESSAGES]}
      >
        <Input placeholder="例如 https://api.anthropic.com" />
      </Form.Item>

      <Form.Item
        name="selected_protocol"
        label="投影使用"
        rules={rules.selectedProtocol}
        extra="只配置一个协议时无需选择；两个都配置时必选，投影按所选协议写入。"
      >
        <Radio.Group
          className="protocol-radios"
          options={PROTOCOL_OPTIONS.map((option) => ({
            ...option,
            disabled: !filledProtocols.includes(option.value),
          }))}
        />
      </Form.Item>

      <Form.Item
        name="api_key"
        label="API Key"
        extra="可选；本地网关可留空。以明文保存在本地配置文件中"
      >
        <Input.Password
          placeholder="留空则不设置"
          autoComplete="new-password"
          visibilityToggle={false}
        />
      </Form.Item>
    </>
  );
}
