import { useState } from "react";
import Modal from "./Modal";
import { ANTHROPIC_MESSAGES, ID_PATTERN, OPENAI_COMPLETIONS } from "../lib/protocols";

/**
 * Create / edit a Provider.
 *
 * `initial` is the existing Provider or `null` for a new one. The id is
 * immutable on edit. The key field never echoes plaintext: while editing a
 * Provider that has a stored Secret, the field shows only a mask, leaving it
 * blank keeps the existing key, typing replaces it, and "清除密钥" removes it.
 */
export default function ProviderForm({ initial, submitting, onSubmit, onCancel }) {
  const editing = initial !== null;
  const hasSavedKey = Boolean(initial?.api_key);

  const [id, setId] = useState(initial?.id ?? "");
  const [description, setDescription] = useState(initial?.description ?? "");
  const [openaiUrl, setOpenaiUrl] = useState(initial?.base_urls?.[OPENAI_COMPLETIONS] ?? "");
  const [anthropicUrl, setAnthropicUrl] = useState(initial?.base_urls?.[ANTHROPIC_MESSAGES] ?? "");
  const [models, setModels] = useState(
    initial?.models?.length ? initial.models.map((m) => ({ ...m })) : [{ id: "", name: "" }],
  );
  const [apiKey, setApiKey] = useState("");
  const [clearKey, setClearKey] = useState(false);
  const [error, setError] = useState(null);

  const setModel = (index, field, value) => {
    setModels((rows) => rows.map((row, i) => (i === index ? { ...row, [field]: value } : row)));
  };

  const handleSubmit = (e) => {
    e.preventDefault();
    setError(null);

    const trimmedId = id.trim();
    if (!ID_PATTERN.test(trimmedId)) {
      setError("id 只能包含字母、数字、-、_、.（且不能为空）");
      return;
    }

    // Drop rows where both fields are empty; a row with a name needs an id.
    const trimmedModels = models
      .map((m) => ({ id: m.id.trim(), name: m.name.trim() }))
      .filter((m) => m.id !== "" || m.name !== "");
    if (trimmedModels.some((m) => m.id === "")) {
      setError("模型行的 id 不能为空");
      return;
    }
    if (trimmedModels.some((m) => !ID_PATTERN.test(m.id))) {
      setError("模型 id 只能包含字母、数字、-、_、.");
      return;
    }

    // api_key semantics: null keeps the stored key (only possible when a key
    // is already saved and the user left the field untouched), "" clears it,
    // otherwise the typed value replaces it.
    const typed = apiKey.trim();
    const apiKeyPayload = hasSavedKey && typed === "" && !clearKey ? null : typed;

    onSubmit({
      id: trimmedId,
      description: description.trim(),
      base_urls: {
        [OPENAI_COMPLETIONS]: openaiUrl.trim() || null,
        [ANTHROPIC_MESSAGES]: anthropicUrl.trim() || null,
      },
      models: trimmedModels,
      api_key: apiKeyPayload,
    });
  };

  return (
    <Modal title={editing ? `编辑 Provider：${initial.id}` : "添加 Provider"} onClose={onCancel}>
      <form className="form" onSubmit={handleSubmit}>
        <label className="field">
          <span className="field-label">id</span>
          <input
            value={id}
            onChange={(e) => setId(e.target.value)}
            placeholder="如 openai"
            disabled={editing}
            autoFocus
          />
          {editing && <span className="field-note">id 不可修改</span>}
        </label>

        <label className="field">
          <span className="field-label">描述</span>
          <input
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="如 OpenAI 官方 API（可留空）"
          />
        </label>

        <label className="field">
          <span className="field-label">base URL · {OPENAI_COMPLETIONS}</span>
          <input
            value={openaiUrl}
            onChange={(e) => setOpenaiUrl(e.target.value)}
            placeholder="https://api.openai.com/v1"
          />
        </label>

        <label className="field">
          <span className="field-label">base URL · {ANTHROPIC_MESSAGES}</span>
          <input
            value={anthropicUrl}
            onChange={(e) => setAnthropicUrl(e.target.value)}
            placeholder="https://api.anthropic.com"
          />
        </label>

        <div className="field">
          <span className="field-label">模型列表</span>
          <div className="model-rows">
            {models.map((model, index) => (
              <div className="model-row" key={index}>
                <input
                  value={model.id}
                  onChange={(e) => setModel(index, "id", e.target.value)}
                  placeholder="模型 id（如 gpt-4o）"
                />
                <input
                  value={model.name}
                  onChange={(e) => setModel(index, "name", e.target.value)}
                  placeholder="展示名（可留空）"
                />
                <button
                  type="button"
                  className="icon-button"
                  onClick={() => setModels((rows) => rows.filter((_, i) => i !== index))}
                  aria-label="删除模型"
                  title="删除模型"
                >
                  ×
                </button>
              </div>
            ))}
          </div>
          <button
            type="button"
            className="ghost-button"
            onClick={() => setModels((rows) => [...rows, { id: "", name: "" }])}
          >
            + 添加模型
          </button>
        </div>

        <label className="field">
          <span className="field-label">密钥</span>
          <input
            type="password"
            autoComplete="new-password"
            value={apiKey}
            onChange={(e) => {
              setApiKey(e.target.value);
              if (clearKey) setClearKey(false);
            }}
            placeholder={hasSavedKey ? "留空则保留已保存的密钥" : "sk-…"}
          />
          {hasSavedKey && !clearKey && (
            <span className="field-note">
              已保存密钥：••••••••（不回显明文），留空则保留
              <button
                type="button"
                className="link-button"
                onClick={() => {
                  setApiKey("");
                  setClearKey(true);
                }}
              >
                清除密钥
              </button>
            </span>
          )}
          {clearKey && <span className="field-note">保存后将清除已保存的密钥</span>}
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
