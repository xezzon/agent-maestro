import {
  APPLY_STATUS_APPLIED,
  APPLY_STATUS_FAILED,
  APPLY_STATUS_SKIPPED,
} from "../../api/plugins";

/**
 * 投影状态的界面呈现：标签文案与配色。
 *
 * 状态词本身是后端的契约（见 `api/plugins.js` 的 `ApplyStatus`），界面只负责
 * 把它翻译成看得懂的一行——三处各写一遍 `===` 判断的做法会让漏掉的状态静默
 * 不显示标签，故此处用一张表一次译全。
 */
const STATUS_PRESENTATION = {
  [APPLY_STATUS_APPLIED]: { label: "已写入", color: "success" },
  [APPLY_STATUS_FAILED]: { label: "失败", color: "error" },
  [APPLY_STATUS_SKIPPED]: { label: "已跳过", color: undefined },
};

/**
 * 取投影状态的界面呈现；未知状态返回 `null`（不渲染标签，而不是渲染一个空标签）。
 * @param {import("../../api/plugins").ApplyStatus} status
 * @returns {{ label: string, color: string|undefined }|null}
 */
export function applyStatusPresentation(status) {
  return STATUS_PRESENTATION[status] ?? null;
}
