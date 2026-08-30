import { ANTHROPIC_MESSAGES, OPENAI_COMPLETIONS } from "../lib/protocols";

export default function ProviderList({ providers, onEdit, onDelete }) {
  if (providers.length === 0) {
    return <p className="empty-hint">还没有 Provider，点击右上角「添加 Provider」开始。</p>;
  }

  return (
    <table className="list-table">
      <thead>
        <tr>
          <th>id</th>
          <th>描述</th>
          <th>协议</th>
          <th>模型</th>
          <th>密钥</th>
          <th className="col-actions">操作</th>
        </tr>
      </thead>
      <tbody>
        {providers.map((provider) => (
          <tr key={provider.id}>
            <td className="mono">{provider.id}</td>
            <td>{provider.description || "—"}</td>
            <td>
              {provider.base_urls?.[OPENAI_COMPLETIONS] ||
              provider.base_urls?.[ANTHROPIC_MESSAGES] ? (
                <div className="badges">
                  {provider.base_urls?.[OPENAI_COMPLETIONS] && <span className="badge">OpenAI</span>}
                  {provider.base_urls?.[ANTHROPIC_MESSAGES] && (
                    <span className="badge">Anthropic</span>
                  )}
                </div>
              ) : (
                "—"
              )}
            </td>
            <td className="models-cell">
              {provider.models.length === 0
                ? "—"
                : provider.models.map((m) => m.name || m.id).join("、")}
            </td>
            <td>{provider.api_key ? "已保存" : "未设置"}</td>
            <td className="row-actions">
              <button type="button" onClick={() => onEdit(provider)}>
                编辑
              </button>
              <button type="button" className="danger" onClick={() => onDelete(provider)}>
                删除
              </button>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
