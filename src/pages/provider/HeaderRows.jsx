import { Button, Flex, Form, Input } from "antd";

/**
 * 自定义 Header 的行列表：一行一个名 / 值，行尾删除、底部添加。
 *
 * 名与值的输入框跟随外层 `Form` 的 `disabled`；增删按钮只看 `saving`，
 * 避免正在提交的那一帧改动行数（显式 `disabled` 会盖掉 `Form` 的 disabled 上下文）。
 *
 * @param {Object} param0
 * @param {{ headerName: Object[], headerValue: Object[] }} param0.rules 名 / 值的规则组
 * @param {boolean} param0.saving 保存进行中：增删禁用
 */
export default function HeaderRows({ rules, saving }) {
  return (
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
  );
}
