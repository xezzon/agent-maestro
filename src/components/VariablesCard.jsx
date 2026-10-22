import { useState } from "react";
import { Button, Card, Form, Input, Typography, message } from "antd";
import { setVariables as saveVariables } from "../api/variables";
import VariableRowsForm from "./VariableRowsForm";
import { globalVariableRules } from "./variableRules";

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

  return (
    <Card title="全局变量">
      {editing ? (
        <VariablesForm
          variables={variables}
          onFinish={(refresh) => {
            setEditing(false);
            if (refresh) {
              onReload();
            }
          }}
        />
      ) : (
        <>
          {names.length === 0 ? (
            <Typography.Text type="secondary">
              尚未定义变量。可在配置值中用 {"${NAME}"}{" "}
              引用变量，投影时替换为默认值
            </Typography.Text>
          ) : (
            <ul className="variable-list">
              {names.map((name) => (
                <li key={name}>
                  <Typography.Text code>{name}</Typography.Text>
                  <Typography.Text className="variable-value">
                    {variables[name]}
                  </Typography.Text>
                </li>
              ))}
            </ul>
          )}
          <div className="card-actions">
            <Button onClick={() => setEditing(true)}>编辑</Button>
          </div>
        </>
      )}
    </Card>
  );
}

/**
 * 全局变量编辑表单：名与默认值必填；规则见 `variableRules`（名仅允许
 * `[A-Za-z0-9_]+`，同表内唯一，大小写敏感）。保存即整包替换全局变量表。
 * @param {Object} param0
 * @param {import("../api/variables").Variables} param0.variables
 * @param {(refresh: boolean) => void} param0.onFinish
 */
function VariablesForm({ variables, onFinish }) {
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);

  // 命令成功即已落盘，由父组件刷新列表。
  async function handleSave(variableValues) {
    setSaving(true);
    try {
      await saveVariables(variableValues);
      onFinish(true);
    } catch (err) {
      message.error(String(err));
    } finally {
      setSaving(false);
    }
  }

  return (
    <VariableRowsForm
      form={form}
      initialRows={Object.entries(variables).map(([name, value]) => ({
        name,
        value,
      }))}
      rules={globalVariableRules(form)}
      nameInput={<Input placeholder="变量名，例如 GATEWAY_ID" />}
      valuePlaceholder="默认值（必填）"
      addLabel="添加变量"
      removeLabel="删除变量"
      saving={saving}
      onSave={handleSave}
      onCancel={() => onFinish(false)}
    />
  );
}
