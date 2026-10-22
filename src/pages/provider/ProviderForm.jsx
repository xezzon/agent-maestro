import { useMemo, useState } from "react";
import {
  Button,
  Checkbox,
  Collapse,
  Flex,
  Form,
  Input,
  InputNumber,
  message,
  Radio,
  Typography,
} from "antd";
import {
  ANTHROPIC_MESSAGES,
  CAPABILITY_IMAGE_IN,
  CAPABILITY_THINKING,
  CAPABILITY_TOOL_USE,
  OPENAI_COMPLETIONS,
  PROVIDER_PROTOCOLS,
  createProvider,
  updateProvider,
} from "../../api/provider";
import FormActions from "../../components/FormActions";
import { providerFormRules } from "./rules";

/** token 上限的预设快捷值：K/M 按十进制（128K = 128000），留空即「未设置」。 */
const LIMIT_PRESETS = [
  { label: "128K", value: 128000 },
  { label: "200K", value: 200000 },
  { label: "1M", value: 1000000 },
];

/** 三项 token 上限的字段名与界面标签。 */
const MODEL_LIMIT_FIELDS = [
  { key: "context_window", label: "上下文窗口" },
  { key: "max_input", label: "单次最大输入" },
  { key: "max_output", label: "最大输出" },
];

/**
 * @param {Object} param0
 * @param {import("../../api/provider").Provider} param0.provider
 * @param {import("../../api/provider").Provider[]} param0.providers 当前存在的 providers，用于检查 slug 冲突
 * @param {(refresh: boolean) => void} param0.onFinish
 * @param {boolean=} param0.disabled 外部（状态开关）进行中时整体禁用表单
 * @param {(busy: boolean) => void=} param0.onBusyChange 上报本表单的保存态
 */
export default function ProviderForm({ provider, providers, onFinish, disabled = false, onBusyChange }) {
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
  const [form] = Form.useForm();
  // 依赖在创建规则前已就绪（form 先声明），不再靠闭包延迟求值；
  // 静态规则组是模块级常量，useMemo 只负责组装，不会每次渲染重建规则。
  const rules = useMemo(
    () => providerFormRules({ form, providers, currentSlug: provider.slug }),
    [form, providers, provider.slug],
  );
  // 实时追踪两个端点槽位，用于禁用「未配置」协议的投影选项。
  const watchedEndpoints = Form.useWatch("base_url", form) ?? {};
  const filledProtocols = PROVIDER_PROTOCOLS.filter(
    (protocol) => (watchedEndpoints[protocol] ?? "").trim() !== "",
  );
  const [saving, setSaving] = useState(false);

  function handleSubmit() {
    submitWith(provider.slug ? updateProvider : createProvider);
  }

  // 命令成功即已落盘，由父组件刷新列表。
  // `enabled` 不是 Form.Item（开关独立于表单），validateFields 不返回它；
  // 显式与 provider 合并回传，避免缺字段被后端默认值静默重置为启用。
  async function submitWith(apiFn) {
    setSaving(true);
    onBusyChange?.(true);
    try {
      const values = await form.validateFields();
      await apiFn({ ...provider, ...values });
      onFinish(true);
    } catch (err) {
      // 校验失败已内联展示，无需重复报错；其余为命令调用失败。
      if (!err?.errorFields) {
        message.error(String(err));
      }
    } finally {
      setSaving(false);
      onBusyChange?.(false);
    }
  }

  return <Form
    form={form}
    layout="vertical"
    disabled={disabled}
    initialValues={provider}
    onFinish={handleSubmit}
  >
    <Form.Item
      name="slug"
      label="Slug"
      extra="slug 创建后不可修改"
      rules={rules.slug}
    >
      <Input disabled={!!provider.slug} />
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

    <Form.Item
      label="自定义 Header"
      extra="可选；随该 Provider 下所有模型与协议共享。值以明文保存在本地配置文件中"
    >
      <Form.List name="custom_header">
        {(fields, { add, remove }) => (
          <div className="header-rows">
            {fields.map((field) => (
              <Flex key={field.key} align="flex-start" gap={8}>
                <Form.Item
                  name={[field.name, "name"]}
                  rules={rules.headerName}
                  className="header-field"
                >
                  <Input placeholder="Header 名，例如 anthropic-version" />
                </Form.Item>
                <Form.Item
                  name={[field.name, "value"]}
                  rules={rules.headerValue}
                  className="header-field"
                >
                  <Input placeholder="Header 值" />
                </Form.Item>
                <Button disabled={saving} onClick={() => remove(field.name)}>
                  删除
                </Button>
              </Flex>
            ))}
            <Button type="dashed" block disabled={saving} onClick={() => add()}>
              添加 Header
            </Button>
          </div>
        )}
      </Form.List>
    </Form.Item>

    <Form.Item label="模型">
      <Form.List name="models">
        {(fields, { add, remove }) => (
          <div className="model-rows">
            {fields.map((field) => (
              <Flex key={field.key} vertical gap={4}>
                <Flex align="flex-start" gap={8} className="model-row">
                  <Form.Item
                    name={[field.name, "id"]}
                    rules={rules.modelId}
                    className="model-field"
                  >
                    <Input placeholder="模型 ID，例如 gpt-4o" />
                  </Form.Item>
                  <Form.Item
                    name={[field.name, "display_name"]}
                    className="model-field"
                  >
                    <Input placeholder="显示名（可选，留空回退显示模型 ID）" />
                  </Form.Item>
                  <Button disabled={saving} onClick={() => remove(field.name)}>
                    删除
                  </Button>
                </Flex>
                <Collapse
                  size="small"
                  ghost
                  items={[
                    {
                      key: "more",
                      label: "更多设置",
                      // 默认收起；forceRender 让面板内的 Form.Item 始终注册，
                      // 否则没展开就保存会丢掉该模型的上限与能力（validateFields 只收已注册字段）。
                      forceRender: true,
                      children: (
                        <>
                          <Form.Item name={[field.name, "capabilities"]}>
                            <Checkbox.Group>
                              <Flex vertical gap={8}>
                                <Flex vertical gap={4}>
                                  <Typography.Text type="secondary">
                                    能力
                                  </Typography.Text>
                                  <Flex gap={16}>
                                    <Checkbox value={CAPABILITY_TOOL_USE}>
                                      工具调用
                                    </Checkbox>
                                    <Checkbox value={CAPABILITY_THINKING}>
                                      推理
                                    </Checkbox>
                                  </Flex>
                                </Flex>
                                <Flex vertical gap={4}>
                                  <Typography.Text type="secondary">
                                    输入模态
                                  </Typography.Text>
                                  <Flex gap={16}>
                                    <Checkbox value={CAPABILITY_IMAGE_IN}>
                                      视觉
                                    </Checkbox>
                                  </Flex>
                                </Flex>
                              </Flex>
                            </Checkbox.Group>
                          </Form.Item>
                          {MODEL_LIMIT_FIELDS.map((limit) => (
                            <Form.Item key={limit.key} label={limit.label}>
                              <Flex gap={4}>
                                <Form.Item
                                  name={[field.name, "limit", limit.key]}
                                  noStyle
                                >
                                  <InputNumber
                                    min={1}
                                    precision={0}
                                    placeholder="未设置"
                                    style={{ width: "100%" }}
                                  />
                                </Form.Item>
                                {LIMIT_PRESETS.map((preset) => (
                                  <Button
                                    key={preset.label}
                                    size="small"
                                    disabled={disabled || saving}
                                    onClick={() =>
                                      form.setFieldValue(
                                        ["models", field.name, "limit", limit.key],
                                        preset.value,
                                      )
                                    }
                                  >
                                    {preset.label}
                                  </Button>
                                ))}
                              </Flex>
                            </Form.Item>
                          ))}
                        </>
                      ),
                    },
                  ]}
                />
              </Flex>
            ))}
            <Button type="dashed" block disabled={saving} onClick={() => add()}>
              添加模型
            </Button>
          </div>
        )}
      </Form.List>
    </Form.Item>

    <FormActions
      saving={saving}
      onCancel={() => onFinish(false)}
      onSave={() => form.submit()}
    />
  </Form>
}
