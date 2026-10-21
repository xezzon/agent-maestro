import { AutoComplete, Button, Flex, Form, Input } from "antd";
import { DeleteOutlined } from "@ant-design/icons";
import FormActions from "../../components/FormActions";
import usePluginConfigSave from "./usePluginConfigSave";

/**
 * 变量覆盖编辑表单：变量名必须已在全局变量表中声明（与后端 set_plugin_config
 * 的校验一致），输入时从全局变量表自动补全；同表内唯一（重复键会被整包替换
 * 静默合并）；覆盖值必填非空白。
 * @param {Object} param0
 * @param {import("../../api/plugins").PluginView} param0.plugin
 * @param {import("../../api/plugins").PluginConfig} param0.config
 * @param {import("../../api/variables").Variables} param0.variables
 * @param {(refresh: boolean) => void} param0.onFinish
 */
export default function PluginVariablesForm({ plugin, config, variables, onFinish }) {
  const [form] = Form.useForm();
  const [saving, saveConfig] = usePluginConfigSave(plugin.source);

  const NAME_RULES = [
    { required: true, message: "请输入变量名" },
    {
      validator: (_, value) => {
        if (!value) return Promise.resolve();
        if (!Object.hasOwn(variables, value)) {
          return Promise.reject(
            new Error(`变量「${value}」未在全局变量表中声明`),
          );
        }
        const occurrences = (form.getFieldValue("variables") ?? []).filter(
          (variable) => variable?.name === value,
        ).length;
        return occurrences > 1
          ? Promise.reject(new Error(`变量「${value}」重复`))
          : Promise.resolve();
      },
    },
  ];
  const VALUE_RULES = [
    { required: true, message: "请输入覆盖值" },
    { whitespace: true, message: "覆盖值不能为空白" },
  ];

  // 命令成功即已落盘，由父组件刷新配置。
  async function handleSubmit(values) {
    await saveConfig(
      config,
      {
        variables: Object.fromEntries(
          values.variables.map((variable) => [variable.name, variable.value]),
        ),
      },
      () => onFinish(true),
    );
  }

  return (
    <Form
      form={form}
      layout="vertical"
      initialValues={{
        variables: Object.entries(config.variables ?? {}).map(([name, value]) => ({
          name,
          value,
        })),
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
                    <AutoComplete
                      options={Object.keys(variables).map((name) => ({
                        value: name,
                      }))}
                      showSearch={{
                        filterOption: (inputValue, option) =>
                          option.value
                            .toUpperCase()
                            .includes(inputValue.toUpperCase()),
                      }}
                      placeholder="全局变量表中已声明的变量名，例如 GATEWAY_ID"
                    />
                  </Form.Item>
                  <Form.Item
                    name={[field.name, "value"]}
                    rules={VALUE_RULES}
                    className="variable-field"
                  >
                    <Input placeholder="覆盖值（必填）" />
                  </Form.Item>
                </Flex>
                <Button
                  type="text"
                  danger
                  aria-label="删除覆盖"
                  title="删除"
                  icon={<DeleteOutlined />}
                  disabled={saving}
                  onClick={() => remove(field.name)}
                />
              </Flex>
            ))}
            <Button type="dashed" block disabled={saving} onClick={() => add()}>
              添加覆盖
            </Button>
          </div>
        )}
      </Form.List>

      <FormActions
        saving={saving}
        onCancel={() => onFinish(false)}
        onSave={() => form.submit()}
      />
    </Form>
  );
}
