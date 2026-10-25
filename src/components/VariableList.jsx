import { Typography } from "antd";

/**
 * 变量表的只读列表：名 → 值。
 *
 * 全局变量表与插件变量覆盖共用这一处渲染：两处的行结构（名 + 值）与空表提示
 * 的位置都相同，只有取值来源与文案不同。
 * @param {Object} param0
 * @param {Record<string, string>} param0.values 变量表（名 → 值）
 * @param {import("react").ReactNode} param0.emptyText 表为空时的提示
 */
export default function VariableList({ values, emptyText }) {
  const names = Object.keys(values);
  if (names.length === 0) {
    return <Typography.Text type="secondary">{emptyText}</Typography.Text>;
  }
  return (
    <ul className="variable-list">
      {names.map((name) => (
        <li key={name}>
          <Typography.Text code>{name}</Typography.Text>
          <Typography.Text className="variable-value">
            {values[name]}
          </Typography.Text>
        </li>
      ))}
    </ul>
  );
}
