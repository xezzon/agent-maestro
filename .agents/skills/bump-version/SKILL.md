---
name: bump-version
description: 升级 Agent Maestro 应用版本号并发起 release 提交时使用。触发词：升级版本、bump version、升到 x.y.z、发版、准备发布、release。覆盖必须同步修改的 4 个文件、需要避开的同名第三方版本号、提交信息约定，以及手动发布流程对分支的要求。
---

# 升级应用版本号

## 何时使用

用户要求升级应用版本（bump / release / 发版），例如「更新应用版本为 0.4.0」。

## 前置：先确认目标版本号

- 版本号由用户给定。若用户只说「升级版本」而没给具体号，先问清楚，不要自行猜 minor 还是 patch。
- 同一版本号不能重复发布：tag 已存在时 tauri-action 不会再建（见 `docs/adr/0009-manual-release-and-per-profile-cache.md`），因此版本号必须严格递增。

## 需要改动的 4 处（且仅这 4 处）

以 0.4.0 为例：

1. `package.json` → 顶层 `"version"`
2. `src-tauri/tauri.conf.json` → `"version"`（**发布产物的版本号与 release tag 取自这里**）
3. `src-tauri/Cargo.toml` → `[package]` 的 `version`
4. `src-tauri/Cargo.lock` → 仅 `name = "agent-maestro"` 那个 `[[package]]` 条目的 `version`

改完后 `src-tauri/Cargo.lock` 应与 manifest 保持同步。

## 不要动的

- `crates/maestro-plugin-sdk/Cargo.toml`（`1.0.0`）：独立版本轴，与 WIT 包 `maestro:plugin` 按 semver 对齐，不是应用版本。
- `plugins/pi/Cargo.toml`（`0.1.0`）：插件自身版本，独立。
- `src-tauri/Cargo.lock` 里第三方 crate 的 `version`：目标值常与若干依赖撞名（例如 `0.4.0` 同时是 `ittapi`、`is-wsl`、`winapi-*-pc-windows-gnu` 的版本），**不要全局查找替换**，只改 `agent-maestro` 那一条。
- `pnpm-lock.yaml`：只记录依赖版本，不含应用版本，无需改动。
- 历史文档里对旧版本号的引用（如 `docs/adr/0010-*.md` 中的「0.3.0 AppImage」）：那是当时产物的记录，不要改。

## 编辑与校验

逐文件精确修改，避免误伤同名版本号。改完后校验 lock 与 manifest 一致：

```bash
cd src-tauri
cargo metadata --format-version 1 --no-deps --locked > /dev/null && echo "Cargo.lock is in sync"
```

`--locked` 在 `Cargo.lock` 需要变动时会直接报错，可作一致性校验；若报错，说明 lock 还没改到位。

## 提交

只暂存这 4 个文件，提交信息沿用既有约定 `chore(release): bump version to X.Y.Z`：

```bash
git add package.json src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/tauri.conf.json
GIT_EDITOR=true git commit -m "chore(release): bump version to 0.4.0"
```

提交前用 `git --no-pager diff --stat` 确认 diff 恰好是 4 个文件、各 1 行。

## 边界：默认不 push、不触发发布

- 发布只能**手动**触发：`.github/workflows/release.yml` 只保留 `workflow_dispatch`，且 job 绑定 GitHub 环境 `release`（分支策略只允许 `main`，并禁用 admin 绕过）。从其它 ref 触发会被拒，tag 也不会创建。
- 因此 bump 提交必须先进入 `main` 才能用于发布。若它落在特性分支上，要提醒用户这一点。
- 默认**不**执行：`git push`、打 tag、`gh workflow run`。除非用户在请求里明确要求。
