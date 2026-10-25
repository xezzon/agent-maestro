# Agent Guide

## Agent skills

### Issue tracker

Issues live in GitHub Issues on `xezzon/agent-maestro`; use the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Canonical labels `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout — `docs/GLOSSARY.md` and `docs/adr/`. See `docs/agents/domain.md`.

## Code conventions

### Logging

Logging goes through the `log` crate (facade: `tauri-plugin-log`, sink: `~/.maestro/logs/`, see ADR 0008). The command layer logs outcome lines via `log_outcome` in `command/mod.rs` (INFO with identifiers on success, ERROR with the reason on failure); the service layer logs key steps at DEBUG. Never log command payloads or `api_key` — identifiers (slug/source) only. Authored log messages are in English; error reasons are the user-facing strings and stay as-is.

The frontend mirrors `console.*` into the same file (`log`→trace and `debug`→debug, dropped at the default INFO level; `info`/`warn`/`error` persist), so never pass credentials to `console.info`, `console.warn` or `console.error`. Uncaught `error`/`unhandledrejection` events are logged separately in `src/main.jsx`; the promises returned by the log plugin are consumed there because a rejected one would re-enter the same listener.

### Tauri commands

Commands are thin shells: pass parameters, call the store, and return the raw domain structure (e.g. `providers` keyed by slug). Type mapping to UI view models happens in the frontend (JS), not in Rust.

One exception: the plugin registry and the projection report are wire-format contracts owned by the command layer (`command/plugin.rs` — `PluginView` / `PluginApplyReport` / `ApplyStatus`). Their shape (`config_dir`, `settings_schema`, `error`, the per-plugin status words) exists for the UI, so it lives with the delivery layer; the plugin service returns its own internal shapes (`PluginRecord` / `PluginOutcome` / `ProjectionStatus`) and the command layer maps them.

### Frontend imports

Frontend-internal imports use the `@/` alias, which points at `src/` (`resolve.alias` in `vite.config.js`, mirrored in `vitest.config.js` so tests resolve the same specifiers the app does, and `paths` in `jsconfig.json` so editors resolve them too). Use it for anything outside the file's own directory: `../../api/provider` chains are hard to read and break when a file moves. Imports of a sibling stay relative (`./formMode`). Third-party packages (`react`, `antd`, …) keep their bare specifiers.
