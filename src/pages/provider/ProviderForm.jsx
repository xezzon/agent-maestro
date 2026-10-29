import { useMemo, useState } from "react";
import { Form } from "antd";
import {
  createProvider,
  initialApiKeyDraft,
  resolveApiKeyAction,
  updateProvider,
} from "@/api/provider";
import FormActions from "@/components/FormActions";
import useAsyncAction from "@/hooks/useAsyncAction";
import ApiKeyField from "./ApiKeyField";
import HeaderRows from "./HeaderRows";
import ModelRows from "./ModelRows";
import ProviderFields from "./ProviderFields";
import { MODE_CREATE, MODE_UPDATE } from "./formMode";
import { providerFormRules } from "./rules";

/** 落盘命令按模式分派：创建与更新的差异只在 slug 的可编辑性与这一处副作用。 */
const SAVE_BY_MODE = {
  [MODE_CREATE]: createProvider,
  [MODE_UPDATE]: updateProvider,
};

/**
 * Provider 表单：只做编排与提交，字段与行列表分别由下面几个组件承接。
 *
 * 创建 / 更新由 `mode` 显式声明，不再靠 slug 是否为空、
 * 也不再靠传空 `providers` 来关闭冲突校验。
 * 保存成功与取消各有回调，调用方不需要解读一个布尔标志。
 *
 * @param {Object} param0
 * @param {"create" | "update"} param0.mode 创建态 slug 可编辑并校验冲突；更新态 slug 只读
 * @param {import("@/api/provider").Provider} param0.provider 初始值（创建态为空白 Provider）
 * @param {import("@/api/provider").Provider[]=} param0.providers 创建态必传：slug 冲突校验
 * @param {() => void} param0.onSaved 已落盘（调用方关表单并刷新列表）
 * @param {() => void} param0.onCancel 取消（不落盘）
 * @param {boolean=} param0.disabled 外部（状态开关）进行中时整体禁用表单
 */
export default function ProviderForm({
  mode,
  provider,
  providers,
  onSaved,
  onCancel,
  disabled = false,
}) {
  const [form] = Form.useForm();
  // 先声明 form 再建规则：端点、header 名与模型 ID 的校验要读表单实时值。
  const rules = useMemo(
    () => providerFormRules({ form, mode, providers }),
    [form, mode, providers],
  );
  const [saving, runSave] = useAsyncAction();
  // 密钥动作是组件本地状态（与 `enabled` 同类，不是 `Form.Item`）：它决定这次
  // 保存对密钥做什么，而密钥值从不进表单状态。未设置态直接给输入框（草稿恒
  // 展开）；已设置 / 明文态先收着，点「更换 API Key」才展开。
  const [apiKeyDraft, setApiKeyDraft] = useState(
    initialApiKeyDraft(provider.api_key_state),
  );

  // 命令成功即已落盘，由调用方刷新列表。
  // `enabled` 不是 Form.Item（开关独立于表单），提交值里没有它；
  // 显式与 provider 合并回传，避免缺字段被后端默认值静默重置为启用。
  // 密钥同样不是 Form.Item：它的动作由本地草稿推出（不变 / 清除 / 覆盖），
  // 负载里带不带 `api_key` 由动作决定。
  // 校验失败由各 Form.Item 内联展示，不会走到这里；这里的错误都是命令调用失败
  // （含密钥链不可用——如实报错，绝不静默）。
  async function handleFinish(values) {
    await runSave(async () => {
      const action = resolveApiKeyAction(provider.api_key_state, apiKeyDraft);
      await SAVE_BY_MODE[mode]({ ...provider, ...values }, action);
      onSaved();
    });
  }

  return (
    <Form
      form={form}
      layout="vertical"
      // 保存期间表单自禁用：保存进行中时字段仍可编辑，用户会以为改的是新值，
      // 实际提交的是旧值。外部（状态开关）进行中时同样整体禁用。
      disabled={disabled || saving}
      initialValues={provider}
      onFinish={handleFinish}
    >
      <ProviderFields mode={mode} rules={rules} />
      <ApiKeyField
        state={provider.api_key_state}
        draft={apiKeyDraft}
        onDraftChange={setApiKeyDraft}
      />
      <HeaderRows rules={rules} saving={saving} />
      <ModelRows rules={rules} saving={saving} disabled={disabled} />

      <FormActions saving={saving} onCancel={onCancel} />
    </Form>
  );
}
