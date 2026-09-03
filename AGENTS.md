# Agent Guide

## Agent skills

### Issue tracker

Issues live in GitHub Issues on `xezzon/agent-maestro`; use the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Canonical labels `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout — `CONTEXT.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.

## Code conventions

### Tauri commands

Commands are thin shells: pass parameters, call the store, and return the raw domain structure (e.g. `providers` keyed by slug). Type mapping to UI view models happens in the frontend (JS), not in Rust.
