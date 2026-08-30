export default function PluginList({ plugins, onToggle, onEdit, onDelete }) {
  if (plugins.length === 0) {
    return <p className="empty-hint">还没有 Plugin，点击右上角「添加 Plugin」开始。</p>;
  }

  return (
    <table className="list-table">
      <thead>
        <tr>
          <th>id</th>
          <th>source</th>
          <th>启用</th>
          <th className="col-actions">操作</th>
        </tr>
      </thead>
      <tbody>
        {plugins.map((plugin) => (
          <tr key={plugin.id} className={plugin.enabled ? "" : "row-disabled"}>
            <td className="mono">{plugin.id}</td>
            <td className="mono">{plugin.source}</td>
            <td>
              <label className="switch" title={plugin.enabled ? "点击禁用" : "点击启用"}>
                <input
                  type="checkbox"
                  checked={plugin.enabled}
                  onChange={() => onToggle(plugin)}
                />
                <span className="slider" />
              </label>
            </td>
            <td className="row-actions">
              <button type="button" onClick={() => onEdit(plugin)}>
                编辑
              </button>
              <button type="button" className="danger" onClick={() => onDelete(plugin)}>
                删除
              </button>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
