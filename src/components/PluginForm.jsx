import { useState } from "react";
import Modal from "./Modal";
import { ID_PATTERN } from "../lib/protocols";

/**
 * Create / edit a Plugin. The id is immutable on edit; `enabled` is the
 * on/off switch persisted with the entry.
 */
export default function PluginForm({ initial, submitting, onSubmit, onCancel }) {
  const editing = initial !== null;

  const [id, setId] = useState(initial?.id ?? "");
  const [source, setSource] = useState(initial?.source ?? "");
  const [enabled, setEnabled] = useState(initial?.enabled ?? true);
  const [error, setError] = useState(null);

  const handleSubmit = (e) => {
    e.preventDefault();
    setError(null);

    const trimmedId = id.trim();
    if (!ID_PATTERN.test(trimmedId)) {
      setError("id 只能包含字母、数字、-、_、.（且不能为空）");
      return;
    }
    if (!source.trim()) {
      setError("Plugin source 不能为空");
      return;
    }

    onSubmit({ id: trimmedId, source: source.trim(), enabled });
  };

  return (
    <Modal title={editing ? `编辑 Plugin：${initial.id}` : "添加 Plugin"} onClose={onCancel}>
      <form className="form" onSubmit={handleSubmit}>
        <label className="field">
          <span className="field-label">id</span>
          <input
            value={id}
            onChange={(e) => setId(e.target.value)}
            placeholder="如 pi"
            disabled={editing}
            autoFocus
          />
          {editing && <span className="field-note">id 不可修改</span>}
        </label>

        <label className="field">
          <span className="field-label">source</span>
          <input
            value={source}
            onChange={(e) => setSource(e.target.value)}
            placeholder="builtin 或插件来源地址"
          />
        </label>

        <label className="field checkbox-field">
          <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} />
          <span>启用</span>
        </label>

        {error && <div className="form-error">{error}</div>}

        <div className="form-actions">
          <button type="button" className="ghost-button" onClick={onCancel} disabled={submitting}>
            取消
          </button>
          <button type="submit" className="primary-button" disabled={submitting}>
            {submitting ? "保存中…" : "保存"}
          </button>
        </div>
      </form>
    </Modal>
  );
}
