# Agent Guide

## Agent skills

### Issue tracker

Issues live in GitHub Issues on `xezzon/agent-maestro`; use the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Canonical labels `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout — `CONTEXT.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.

## Code conventions

### Logging

Logging goes through the `log` crate (facade: `tauri-plugin-log`, sink: `~/.maestro/logs/`, see ADR 0008). The command layer logs outcome lines via `log_outcome` in `command/mod.rs` (INFO with identifiers on success, ERROR with the reason on failure); the service layer logs key steps at DEBUG. Never log command payloads or `api_key` — identifiers (slug/source) only. Authored log messages are in English; error reasons are the user-facing strings and stay as-is.

### Tauri commands

Commands are thin shells: pass parameters, call the store, and return the raw domain structure (e.g. `providers` keyed by slug). Type mapping to UI view models happens in the frontend (JS), not in Rust.
