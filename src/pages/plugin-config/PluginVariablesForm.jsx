import { AutoComplete, Form } from "antd";
import VariableRowsForm from "@/components/VariableRowsForm";
import { overrideVariableRules } from "@/components/variableRules";
import usePluginConfigSave from "./usePluginConfigSave";

/**
 * 变量覆盖编辑表单：变量名必须已在全局变量表中声明（与后端 set_plugin_config
 * 的校验一致），输入时从全局变量表自动补全；同表内唯一，覆盖值必填非空白
 * （规则见 `variableRules`）。保存整包写回插件配置。
 * @param {Object} param0
 * @param {import("@/api/plugins").PluginView} param0.plugin
 * @param {import("@/api/plugins").PluginConfig} param0.config
 * @param {import("@/api/variables").Variables} param0.variables
 * @param {(refresh: boolean) => void} param0.onFinish
 */
export default function PluginVariablesForm({
  plugin,
  config,
  variables,
  onFinish,
}) {
  const [form] = Form.useForm();
  const [saving, saveConfig] = usePluginConfigSave(plugin.source);

  // 命令成功即已落盘，由父组件刷新配置。
  async function handleSave(record) {
    await saveConfig(config, { variables: record }, () => onFinish(true));
  }

  return (
    <VariableRowsForm
      form={form}
      initialRows={Object.entries(config.variables ?? {}).map(
        ([name, value]) => ({
          name,
          value,
        }),
      )}
      rules={overrideVariableRules({ form, variables })}
      nameInput={
        <AutoComplete
          options={Object.keys(variables).map((name) => ({ value: name }))}
          showSearch={{
            filterOption: (inputValue, option) =>
              option.value.toUpperCase().includes(inputValue.toUpperCase()),
          }}
          placeholder="全局变量表中已声明的变量名，例如 GATEWAY_ID"
        />
      }
      valuePlaceholder="覆盖值（必填）"
      addLabel="添加覆盖"
      removeLabel="删除覆盖"
      saving={saving}
      onSave={handleSave}
      onCancel={() => onFinish(false)}
    />
  );
}
