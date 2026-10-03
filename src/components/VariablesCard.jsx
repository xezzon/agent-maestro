import { useState } from "react";
import { DeleteOutlined } from "@ant-design/icons";
import { Button, Card, Flex, Form, Input, Typography, message } from "antd";
import { setVariables as saveVariables } from "../api/variables";

/**
 * 全局变量表卡片（issue #90）：维护全局配置里的变量表（名 → 默认值，ADR 0015）。
 * 数据由所在页面持有（`variables` + `onReload`），可在多个页面复用。
 * 只读态展示名 → 默认值；编辑态整表增删改，保存即写回全局配置。
 * @param {Object} param0
 * @param {import("../api/variables").Variables} param0.variables
 * @param {() => void} param0.onReload
 */
export default function VariablesCard({ variables, onReload }) {
  const [editing, setEditing] = useState(false);
  const names = Object.keys(variables);

  return <Card title="全局变量">
    {editing
      ? <VariablesForm
        variables={variables}
        onFinish={(refresh) => {
          setEditing(false);
          if (refresh) {
            onReload();
          }
        }}
      />
      : <>
        {names.length === 0 ? (
          <Typography.Text type="secondary">
            尚未定义变量。可在配置值中用 {"${NAME}"} 引用变量，投影时替换为默认值
          </Typography.Text>
        ) : (
          <ul className="variable-list">
            {names.map((name) => (
              <li key={name}>
                <Typography.Text code>{name}</Typography.Text>
                <Typography.Text className="variable-value">{variables[name]}</Typography.Text>
              </li>
            ))}
          </ul>
        )}
        <div className="card-actions">
          <Button onClick={() => setEditing(true)}>
            编辑
          </Button>
        </div>
      </>}
  </Card>;
}

/**
 * 全局变量编辑表单：名与默认值必填；变量名仅允许 `[A-Za-z0-9_]+`（ADR 0014），
 * 同表内唯一（大小写敏感）。保存即整包替换全局变量表。
 * @param {Object} param0
 * @param {import("../api/variables").Variables} param0.variables
 * @param {(refresh: boolean) => void} param0.onFinish
 */
function VariablesForm({ variables, onFinish }) {
  const NAME_PATTERN = /^[A-Za-z0-9_]+$/;
  // 唯一性校验依赖当前表单内全部变量行的实时值。
  const NAME_RULES = [
    { required: true, message: "请输入变量名" },
    { pattern: NAME_PATTERN, message: "变量名仅允许字母、数字和下划线" },
    {
      validator: (_, value) => {
        if (!value) return Promise.resolve();
        const occurrences = (form.getFieldValue("variables") ?? []).filter(
          (variable) => variable?.name === value,
        ).length;
        return occurrences > 1
          ? Promise.reject(new Error(`变量名「${value}」重复`))
          : Promise.resolve();
      },
    },
  ];
  const VALUE_RULES = [
    { required: true, message: "请输入默认值" },
    { whitespace: true, message: "默认值不能为空白" },
  ];

  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);

  // 命令成功即已落盘，由父组件刷新列表。
  async function handleSubmit(values) {
    setSaving(true);
    try {
      await saveVariables(Object.fromEntries(
        values.variables.map((variable) => [variable.name, variable.value]),
      ));
      onFinish(true);
    } catch (err) {
      message.error(String(err));
    } finally {
      setSaving(false);
    }
  }

  return <Form
    form={form}
    layout="vertical"
    initialValues={{
      variables: Object.entries(variables).map(([name, value]) => ({ name, value })),
    }}
    onFinish={handleSubmit}
  >
    <Form.List name="variables">
      {(fields, { add, remove }) => (
        <div className="variable-rows">
          {fields.map((field) => (
            <Flex key={field.key} align="center" gap={8} className="variable-row">
              <Flex vertical gap={8} className="variable-fields">
                <Form.Item
                  name={[field.name, "name"]}
                  rules={NAME_RULES}
                  className="variable-name"
                >
                  <Input placeholder="变量名，例如 GATEWAY_ID" />
                </Form.Item>
                <Form.Item
                  name={[field.name, "value"]}
                  rules={VALUE_RULES}
                  className="variable-field"
                >
                  <Input placeholder="默认值（必填）" />
                </Form.Item>
              </Flex>
              <Button
                type="text"
                danger
                aria-label="删除变量"
                title="删除"
                icon={<DeleteOutlined />}
                disabled={saving}
                onClick={() => remove(field.name)}
              />
            </Flex>
          ))}
          <Button type="dashed" block disabled={saving} onClick={() => add()}>
            添加变量
          </Button>
        </div>
      )}
    </Form.List>

    <div className="card-actions">
      <Button disabled={saving} onClick={() => onFinish(false)}>
        取消
      </Button>
      <Button
        type="primary"
        loading={saving}
        onClick={() => form.submit()}
      >
        保存
      </Button>
    </div>
  </Form>;
}
