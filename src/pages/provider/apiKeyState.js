import { API_KEY_PLAINTEXT, API_KEY_SET, API_KEY_UNSET } from "@/api/provider";

/**
 * 凭证状态的界面呈现：标签文案、输入框提示与「明文待迁移」告警。
 *
 * 状态词本身是后端的契约（见 `api/provider.js` 的 `ApiKeyState`），界面只负责
 * 把它翻译成看得懂的一行——只读卡片与编辑表单各写一遍 `===` 判断的做法会让
 * 漏掉的状态静默不显示标签，故此处用一张表一次译全。
 *
 * `plaintext`（真值仍在配置文件里）与 `set`（真值在密钥链里）**标签相同**：
 * 两者都是「已设置」；区别由「明文待迁移」告警单独点出。
 *
 * `hasCredential` 说的是「有没有一条已配的凭证」：有才谈得上更换与清除
 * （未设置态没有旧值可清，故不提供「取消更换」，输入框也一直摆着）。
 */
const API_KEY_PRESENTATION = {
  [API_KEY_UNSET]: {
    label: "未设置",
    pendingMigration: false,
    hasCredential: false,
    placeholder: "留空则不设置",
  },
  [API_KEY_SET]: {
    label: "已设置",
    pendingMigration: false,
    hasCredential: true,
    placeholder: "留空即清除当前密钥",
  },
  [API_KEY_PLAINTEXT]: {
    label: "已设置",
    pendingMigration: true,
    hasCredential: true,
    placeholder: "留空即清除当前密钥",
  },
};

/**
 * 取凭证状态的界面呈现；未知状态返回 `null`（不渲染标签，而不是渲染一个空标签）。
 * @param {import("@/api/provider").ApiKeyState} state
 * @returns {{ label: string, pendingMigration: boolean, hasCredential: boolean, placeholder: string }|null}
 */
export function apiKeyStatePresentation(state) {
  return API_KEY_PRESENTATION[state] ?? null;
}
