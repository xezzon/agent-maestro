import { Button, Form } from "antd";
import ModelRow from "./ModelRow";

/**
 * 模型行列表：一行一个模型，底部为添加按钮；行内结构见 `ModelRow`。
 *
 * @param {Object} param0
 * @param {{ modelId: Object[] }} param0.rules 模型 ID 的规则组
 * @param {boolean} param0.saving 保存进行中：增删禁用
 * @param {boolean} param0.disabled 外部（状态开关）进行中：上限预设禁用
 */
export default function ModelRows({ rules, saving, disabled }) {
  return (
    <Form.Item label="模型">
      <Form.List name="models">
        {(fields, { add, remove }) => (
          <div className="model-rows">
            {fields.map((field) => (
              <ModelRow
                key={field.key}
                field={field}
                rules={rules}
                saving={saving}
                disabled={disabled}
                onRemove={() => remove(field.name)}
              />
            ))}
            <Button type="dashed" block disabled={saving} onClick={() => add()}>
              添加模型
            </Button>
          </div>
        )}
      </Form.List>
    </Form.Item>
  );
}
