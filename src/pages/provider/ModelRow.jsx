import {
  Button,
  Checkbox,
  Collapse,
  Flex,
  Form,
  Input,
  InputNumber,
  Typography,
} from "antd";
import {
  CAPABILITY_IMAGE_IN,
  CAPABILITY_LABELS,
  CAPABILITY_THINKING,
  CAPABILITY_TOOL_USE,
} from "../../api/provider";

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

/** 能力复选框的展示分组；分组只影响界面，落盘仍是一维枚举串。 */
const CAPABILITY_GROUPS = [
  {
    label: "能力",
    options: [
      {
        value: CAPABILITY_TOOL_USE,
        label: CAPABILITY_LABELS[CAPABILITY_TOOL_USE],
      },
      {
        value: CAPABILITY_THINKING,
        label: CAPABILITY_LABELS[CAPABILITY_THINKING],
      },
    ],
  },
  {
    label: "输入模态",
    options: [
      {
        value: CAPABILITY_IMAGE_IN,
        label: CAPABILITY_LABELS[CAPABILITY_IMAGE_IN],
      },
    ],
  },
];

/**
 * 一条模型：模型 ID（必填，同一 Provider 内唯一）与显示名（可选），
 * 折叠面板里是能力与三项 token 上限（见 `ModelAdvancedFields`）。
 *
 * @param {Object} param0
 * @param {{ key: number, name: number }} param0.field `Form.List` 给出的行描述
 * @param {{ modelId: Object[] }} param0.rules 模型 ID 的规则组
 * @param {boolean} param0.saving 保存进行中：删除与上限预设禁用
 * @param {boolean} param0.disabled 外部（状态开关）进行中：上限预设禁用
 * @param {() => void} param0.onRemove 删除本行
 */
export default function ModelRow({ field, rules, saving, disabled, onRemove }) {
  return (
    <Flex vertical gap={4}>
      <Flex align="flex-start" gap={8} className="model-row">
        <Form.Item
          name={[field.name, "id"]}
          rules={rules.modelId}
          className="model-field"
        >
          <Input placeholder="模型 ID，例如 gpt-4o" />
        </Form.Item>
        <Form.Item name={[field.name, "display_name"]} className="model-field">
          <Input placeholder="显示名（可选，留空回退显示模型 ID）" />
        </Form.Item>
        <Button disabled={saving} onClick={onRemove}>
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
              <ModelAdvancedFields
                field={field}
                saving={saving}
                disabled={disabled}
              />
            ),
          },
        ]}
      />
    </Flex>
  );
}

/**
 * 模型行的折叠面板内容：能力勾选与三项 token 上限。
 * 上限的预设按钮直接把值写进同名字段，所见即落盘值。
 *
 * @param {Object} param0
 * @param {{ name: number }} param0.field `Form.List` 给出的行描述
 * @param {boolean} param0.saving 保存进行中：预设按钮禁用
 * @param {boolean} param0.disabled 外部（状态开关）进行中：预设按钮禁用
 */
function ModelAdvancedFields({ field, saving, disabled }) {
  const form = Form.useFormInstance();

  return (
    <>
      <Form.Item name={[field.name, "capabilities"]}>
        <Checkbox.Group>
          <Flex vertical gap={8}>
            {CAPABILITY_GROUPS.map((group) => (
              <Flex key={group.label} vertical gap={4}>
                <Typography.Text type="secondary">
                  {group.label}
                </Typography.Text>
                <Flex gap={16}>
                  {group.options.map((option) => (
                    <Checkbox key={option.value} value={option.value}>
                      {option.label}
                    </Checkbox>
                  ))}
                </Flex>
              </Flex>
            ))}
          </Flex>
        </Checkbox.Group>
      </Form.Item>
      {MODEL_LIMIT_FIELDS.map((limit) => (
        <Form.Item key={limit.key} label={limit.label}>
          <Flex gap={4}>
            <Form.Item name={[field.name, "limit", limit.key]} noStyle>
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
  );
}
