/**
 * Provider 表单的两种模式：显式声明创建 / 更新，取代
 * 「slug 是否为空」与「providers 是否为空数组」这类隐式信号。
 */

/** 创建态：slug 可编辑，必须避开已有 Provider 的 slug。 */
export const MODE_CREATE = "create";

/** 更新态：slug 只读，不做冲突校验。 */
export const MODE_UPDATE = "update";
