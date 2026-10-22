import { DeleteOutlined } from "@ant-design/icons";
import { Button, Flex, Form, Input } from "antd";
import FormActions from "./FormActions";

/**
 * 变量行表单（issue #123）：全局变量表与插件变量覆盖共用这一处行实现。
 * 调用方只传差异项——初始行、名 / 值规则、名输入控件、保存回调与文案；
 * 行的结构（名 / 值 / 删除）、添加按钮与取消 / 保存操作块都在此收口。
 *
 * 提交时把行转成 `{ 名: 值 }` 记录交给 `onSave`（整包替换该段）；
 * 落盘、成功提示与错误提示由调用方负责，两处的保存路径本就不同
 * （全局配置 vs 插件配置）。
 *
 * @param {Object} param0
 * @param {{ getFieldValue: (name: any) => any, submit: () => void }} param0.form
 *   调用方持有：唯一性等规则要先拿到它再建
 * @param {Array<{ name: string, value: string }>} param0.initialRows 初始行
 * @param {{ name: Object[], value: Object[] }} param0.rules 名 / 值的校验规则组
 * @param {import("react").ReactNode} param0.nameInput 名输入控件（`Input` 或带补全的 `AutoComplete`）
 * @param {string} param0.valuePlaceholder 值输入框的占位文案
 * @param {string} param0.addLabel 添加按钮文案
 * @param {string} param0.removeLabel 删除按钮的无障碍名与 tooltip
 * @param {boolean} param0.saving 保存进行中：增删与取消 / 保存都禁用
 * @param {(variables: Record<string, string>) => Promise<void>} param0.onSave 保存整表
 * @param {() => void} param0.onCancel 取消（不落盘）
 */
export default function VariableRowsForm({
  form,
  initialRows,
  rules,
  nameInput,
  valuePlaceholder,
  addLabel,
  removeLabel,
  saving,
  onSave,
  onCancel,
}) {
  async function handleFinish(values) {
    await onSave(
      Object.fromEntries(
        values.variables.map((variable) => [variable.name, variable.value]),
      ),
    );
  }

  return (
    <Form
      form={form}
      layout="vertical"
      initialValues={{ variables: initialRows }}
      onFinish={handleFinish}
    >
      <Form.List name="variables">
        {(fields, { add, remove }) => (
          <div className="variable-rows">
            {fields.map((field) => (
              <Flex
                key={field.key}
                align="center"
                gap={8}
                className="variable-row"
              >
                <Flex vertical gap={8} className="variable-fields">
                  <Form.Item
                    name={[field.name, "name"]}
                    rules={rules.name}
                    className="variable-name"
                  >
                    {nameInput}
                  </Form.Item>
                  <Form.Item
                    name={[field.name, "value"]}
                    rules={rules.value}
                    className="variable-field"
                  >
                    <Input placeholder={valuePlaceholder} />
                  </Form.Item>
                </Flex>
                <Button
                  type="text"
                  danger
                  aria-label={removeLabel}
                  title={removeLabel}
                  icon={<DeleteOutlined />}
                  disabled={saving}
                  onClick={() => remove(field.name)}
                />
              </Flex>
            ))}
            <Button type="dashed" block disabled={saving} onClick={() => add()}>
              {addLabel}
            </Button>
          </div>
        )}
      </Form.List>

      <FormActions
        saving={saving}
        onCancel={onCancel}
        onSave={() => form.submit()}
      />
    </Form>
  );
}
