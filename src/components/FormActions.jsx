import { Button } from "antd";

/**
 * 表单的「取消 / 保存」操作块：Provider 编辑、全局变量、插件变量覆盖三处共用。
 * 只负责这一对动作；卡片上的编辑 / 删除等其它操作块不在此列。
 *
 * 保存走调用方传入的回调（表单用 `form.submit`），而不是组件内的 submit 按钮：
 * Ant Design 的 `form.submit()` 先校验再触发 `onFinish`，校验失败时错误仍由各
 * Form.Item 内联展示，不会走到提交逻辑。
 *
 * @param {Object} param0
 * @param {boolean} param0.saving 保存进行中：取消与保存都禁用，保存按钮显示 loading
 * @param {() => void} param0.onCancel 取消（不落盘）
 * @param {() => void} param0.onSave 保存（通常是 `() => form.submit()`）
 */
export default function FormActions({ saving, onCancel, onSave }) {
  return (
    <div className="card-actions">
      <Button disabled={saving} onClick={onCancel}>
        取消
      </Button>
      <Button type="primary" loading={saving} onClick={onSave}>
        保存
      </Button>
    </div>
  );
}
