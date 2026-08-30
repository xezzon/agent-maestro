// Protocol identifiers used as keys of a Provider's base_urls map.
// Keep in sync with the serde renames in src-tauri/src/config.rs.

export const OPENAI_COMPLETIONS = "openai-completions";
export const ANTHROPIC_MESSAGES = "anthropic-messages";

// Mirrors `is_valid_id` in src-tauri/src/config.rs: ids may contain ASCII
// alphanumerics, `-`, `_`, and `.`. Kept in sync so the forms reject bad ids
// before round-tripping to the backend.
export const ID_PATTERN = /^[A-Za-z0-9._-]+$/;
