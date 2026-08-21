# Agent Guide

## External specifications

- MCP 开发（server 接入、传输、配置字段、tools/resources/prompts 等协议行为）遵循 [Model Context Protocol](https://modelcontextprotocol.io/) 官方规范。
- Skill 开发（`SKILL.md`、frontmatter、附带资源、打包与分发）遵循 [Agent Skills Specification](https://agentskills.io/specification)。

## Agent skills

### Issue tracker

Issues live in GitHub Issues on `xezzon/agent-maestro`; use the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Canonical labels `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout — `CONTEXT.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.
