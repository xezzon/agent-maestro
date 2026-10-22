import { Button, Card } from "antd";
import RjsfForm from "@rjsf/antd";
import { customizeValidator } from "@rjsf/validator-ajv8";
import Ajv2020 from "ajv/dist/2020";
import usePluginConfigSave from "./usePluginConfigSave";

/**
 * 表单校验器：settings_schema 声明的方言是 draft 2020-12（ADR 0017），而
 * @rjsf/validator-ajv8 默认的 Ajv 只带 draft-07 的 meta-schema，编译这样的 schema
 * 会抛 `no schema with key or ref "…/draft/2020-12/schema"`（保存时表现为校验失败）；
 * 换成 Ajv2020 后 schema 里的 `$schema` 有对应的 meta-schema 可解析。
 */
const validator = customizeValidator({ AjvClass: Ajv2020 });

/**
 * 插件表单：按 manifest 的 settings_schema（内联 JSON Schema，ADR 0017）用
 * @rjsf/antd 渲染，输入期校验由 rjsf 按 schema 完成（宿主不校验 form）。
 * 保存整包写回插件配置（form 段写入提交的表单数据，variables 段原样带回）。
 * @param {Object} param0
 * @param {import("@/api/plugins").PluginView} param0.plugin
 * @param {import("@/api/plugins").PluginConfig} param0.config
 * @param {() => void} param0.onReload
 */
export default function PluginForm({ plugin, config, onReload }) {
  const [saving, saveConfig] = usePluginConfigSave(plugin.source);

  async function handleSubmit({ formData }) {
    await saveConfig(config, { form: formData ?? null }, onReload);
  }

  return (
    <Card title="表单">
      {/* noHtml5Validate：schema 声明了 required 时 rjsf 会把 required 传给 <input>，
          浏览器的原生校验会先短路 submit，rjsf 自己的错误列表与字段提示就不会跑；
          原生气泡由 webview 决定是否显示，故关掉它，改成与其它表单一致的常驻提示。 */}
      <RjsfForm
        schema={plugin.settings_schema}
        formData={config.form ?? undefined}
        validator={validator}
        noHtml5Validate
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
