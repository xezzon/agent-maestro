import { useCallback, useEffect, useState } from "react";
import {
  Alert,
  Button,
  Card,
  Empty,
  Flex,
  Form,
  Input,
  Spin,
  Tabs,
  Typography,
  message,
} from "antd";
import { DeleteOutlined } from "@ant-design/icons";
import RjsfForm from "@rjsf/antd";
import validator from "@rjsf/validator-ajv8";
import { getPluginConfig, setPluginConfig } from "./api/plugins";
import { listVariables } from "./api/variables";

/**
 * 插件配置页：分「变量覆盖」「表单」两个 Tab。表单按 manifest 的
 * settings_schema（ADR 0017）渲染，写入插件配置的 form 段。
 * 保存只写该插件的配置（ADR 0018），不触发投影——投影仍由 Provider 页的
 * 「应用到工具」显式发起。
 * @param {Object} param0
 * @param {import("./api/plugins").PluginView} param0.plugin
 */
export default function PluginConfigPage({ plugin }) {
  const [config, setConfig] = useState(null);
  // 全局变量表（ADR 0015）：覆盖项的名字校验依赖它（声明的真相源）。
  const [variables, setVariables] = useState({});
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(null);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const [config, variables] = await Promise.all([
        getPluginConfig(plugin.source),
        listVariables(),
      ]);
      setConfig(config);
      setVariables(variables);
      setLoadError(null);
    } catch (err) {
      setLoadError(String(err));
    } finally {
      setLoading(false);
    }
  }, [plugin.source]);

  useEffect(() => {
    reload();
  }, [reload]);

  if (loadError) {
    return (
      <div className="page">
        <PageHeader plugin={plugin} />
        <Alert
          type="error"
          showIcon
          title="插件配置加载失败"
          description={<pre className="error-detail">{loadError}</pre>}
        />
      </div>
    );
  }

  return (
    <div className="page">
      <PageHeader plugin={plugin} />
      {loading && config === null ? (
        <div className="page-loading">
          <Spin />
        </div>
      ) : (
        <Tabs
          defaultActiveKey="variables"
          items={[
            {
              key: "variables",
              label: "变量覆盖",
              children: (
                <PluginVariablesCard
                  plugin={plugin}
                  config={config}
                  variables={variables}
                  onReload={reload}
                />
              ),
            },
            {
              key: "form",
              label: "表单",
              children: plugin.settings_schema ? (
                <PluginSettingsForm
                  plugin={plugin}
                  config={config}
                  onReload={reload}
                />
              ) : (
                <Empty description="该插件没有需要填写的配置项" />
              ),
            },
          ]}
        />
      )}
    </div>
  );
}

/**
 * 页头：插件标识（加载失败与正常两个分支共用）。
 * @param {Object} param0
 * @param {import("./api/plugins").PluginView} param0.plugin
 */
function PageHeader({ plugin }) {
  return (
    <div className="page-header">
      <Typography.Title level={4} style={{ margin: 0 }}>
        {plugin.id || plugin.source}
      </Typography.Title>
    </div>
  );
}

/**
 * 插件配置保存的共享样板：saving 状态 + 整包写回 + 失败 message.error。
 * 返回 [saving, save]；save(config, patch, onSaved) 把 patch 并入 config
 * 整包写回该插件的配置（ADR 0018），成功后调用 onSaved（由调用方决定是否刷新）。
 * @param {string} source 插件来源（条目身份，同 PluginEntry::source）
 * @returns {[boolean, (config: import("./api/plugins").PluginConfig, patch: Object, onSaved: () => void) => Promise<void>]}
 */
function usePluginConfigSave(source) {
  const [saving, setSaving] = useState(false);
  async function save(config, patch, onSaved) {
    setSaving(true);
    try {
      await setPluginConfig(source, { ...config, ...patch });
      onSaved();
    } catch (err) {
      message.error(String(err));
    } finally {
      setSaving(false);
    }
  }
  return [saving, save];
}

/**
 * 变量覆盖卡片：该插件对全局变量的覆盖（名 → 覆盖值，ADR 0018）。
 * 数据由所在页面持有（config + 全局变量表 variables + onReload）。
 * 只读态展示覆盖项；编辑态按名增删改，保存整包写回插件配置（form 段原样带回）。
 * @param {Object} param0
 * @param {import("./api/plugins").PluginView} param0.plugin
 * @param {import("./api/plugins").PluginConfig} param0.config
 * @param {import("./api/variables").Variables} param0.variables
 * @param {() => void} param0.onReload
 */
function PluginVariablesCard({ plugin, config, variables, onReload }) {
  const [editing, setEditing] = useState(false);
  const names = Object.keys(config.variables);

  return (
    <Card title="变量覆盖">
      {editing ? (
        <PluginVariablesForm
          plugin={plugin}
          config={config}
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
              尚未覆盖任何变量。覆盖后投影该插件时，覆盖值替代全局变量表中的默认值
            </Typography.Text>
          ) : (
            <ul className="variable-list">
              {names.map((name) => (
                <li key={name}>
                  <Typography.Text code>{name}</Typography.Text>
                  <Typography.Text className="variable-value">
                    {config.variables[name]}
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
 * 变量覆盖编辑表单：变量名必须已在全局变量表中声明（与后端 set_plugin_config
 * 的校验一致），同表内唯一（重复键会被整包替换静默合并）；覆盖值必填非空白。
 * @param {Object} param0
 * @param {import("./api/plugins").PluginView} param0.plugin
 * @param {import("./api/plugins").PluginConfig} param0.config
 * @param {import("./api/variables").Variables} param0.variables
 * @param {(refresh: boolean) => void} param0.onFinish
 */
function PluginVariablesForm({ plugin, config, variables, onFinish }) {
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
        variables: Object.entries(config.variables).map(([name, value]) => ({
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
                    <Input placeholder="全局变量表中已声明的变量名，例如 GATEWAY_ID" />
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
    </Form>
  );
}

/**
 * 插件表单：按 manifest 的 settings_schema（内联 JSON Schema，ADR 0017）用
 * @rjsf/antd 渲染，输入期校验由 rjsf 按 schema 完成（宿主不校验 form）。
 * 保存整包写回插件配置（form 段写入提交的表单数据，variables 段原样带回）。
 * @param {Object} param0
 * @param {import("./api/plugins").PluginView} param0.plugin
 * @param {import("./api/plugins").PluginConfig} param0.config
 * @param {() => void} param0.onReload
 */
function PluginSettingsForm({ plugin, config, onReload }) {
  const [saving, saveConfig] = usePluginConfigSave(plugin.source);

  async function handleSubmit({ formData }) {
    await saveConfig(config, { form: formData ?? null }, onReload);
  }

  return (
    <Card title="表单">
      <RjsfForm
        schema={plugin.settings_schema}
        formData={config.form ?? undefined}
        validator={validator}
        onSubmit={handleSubmit}
      >
        <div className="card-actions">
          <Button type="primary" htmlType="submit" loading={saving}>
            保存
          </Button>
        </div>
      </RjsfForm>
    </Card>
  );
}
