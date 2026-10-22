import { Button } from "antd";

/**
 * 表单的「取消 / 保存」操作块：Provider 编辑、全局变量、插件变量覆盖三处共用。
 * 只负责这一对动作；卡片上的编辑 / 删除等其它操作块不在此列。
 *
 * 保存按钮是所在表单的提交按钮（`htmlType="submit"`）：点击与回车走同一条路径
 * ——Ant Design 先校验再触发 `onFinish`，校验失败时错误仍由各 Form.Item 内联展示，
 * 不会走到提交逻辑。表单里没有第二个提交按钮，回车才能触发提交。
 *
 * @param {Object} param0
 * @param {boolean} param0.saving 保存进行中：取消与保存都禁用，保存按钮显示 loading
 * @param {() => void} param0.onCancel 取消（不落盘）
 */
export default function FormActions({ saving, onCancel }) {
  return (
    <div className="card-actions">
      <Button disabled={saving} onClick={onCancel}>
        取消
      </Button>
      <Button type="primary" htmlType="submit" loading={saving}>
        保存
      </Button>
    </div>
  );
}
