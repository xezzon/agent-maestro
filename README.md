# Agent Maestro

> Agent 工具的统一配置中心：Provider、模型、MCP、Skills，配置一次，处处生效。

Agent Maestro 是一个跨平台桌面应用，用于统一管理 [Pi](https://pi.dev)、[OpenCode](https://opencode.ai)、[Zed](https://zed.dev) 等 Agent 工具的配置。Provider、模型、MCP Server 和 Skills 在 Maestro 中集中配置后，通过投影（projection）写入各工具的配置文件，无需在各工具中分别配置。

> ⚠️ **项目状态：早期开发中。** 当前还没有发布版本，功能仍在实现中。

## 功能

- **统一管理 Provider 和模型**：集中维护模型 Provider、模型和 API Key，并投影到各 Agent 工具。API Key 保存在系统密钥链（macOS Keychain / Windows Credential Manager / Linux Secret Service）中，**仅存储在本地，不会上传到任何服务器**。
- **统一管理 MCP 和 Skills**：遵循 [MCP 规范](https://modelcontextprotocol.io/) 和 [Agent Skills 规范](https://agentskills.io/specification)。MCP Server 和 Skills 定义一次即可在各工具间使用；支持多个 Profile——每个 Profile 是一组启用的 MCP 和 Skills 的命名集合，可绑定到不同的 Agent 工具（如「工作」「个人」「实验」）。
- **配置跨设备同步**：在多台电脑之间同步 Maestro 的配置（规划中，第一期未实现）。

## 支持的 Agent 工具

- [ ] [Pi](https://pi.dev)
- [ ] [OpenCode](https://opencode.ai)
- [ ] [Zed](https://zed.dev)
- [ ] ...

## 使用流程

1. **添加 Provider**：填入模型 Provider 的地址、API Key 和可用模型。第一期不内置 Provider 目录，需手动添加所使用的 Provider。
2. **配置 MCP 和 Skills**：添加需要使用的 MCP Server；从 Git 仓库安装 Skills 到 Maestro 的技能库。
3. **创建 Profile**：为不同场景创建 Profile，勾选启用的 MCP 和 Skills，并把 Profile 绑定到对应的 Agent 工具。
4. **执行投影**：Maestro 将配置写入各工具的配置文件，之后即可在对应的 Agent 工具中使用。

## 使用须知

- **配置以 Maestro 为准**：请始终在 Maestro 中修改配置后重新投影。直接在 Agent 工具里手动改动的配置不会被 Maestro 读取，下次投影时可能被覆盖。
- **Skills 以符号链接部署**：安装的 Skills 会通过符号链接出现在各工具的 Skills 目录中（Windows 上使用目录联接 Junction，无需管理员权限），请勿手动删除这些链接。
- 对各 Agent 工具的适配由内置插件（Plugin）提供，第一期包含 Pi、OpenCode、Zed 三个插件。

## 支持的平台

基于 Tauri v2，Maestro 提供桌面端应用，支持以下平台：

- **Windows**: Windows 10 及以上（x86_64）。提供安装包（NSIS）。
- **macOS**: Intel 与 Apple Silicon（aarch64）。提供 `.app` / `.dmg`。
- **Linux**: x86_64、aarch64。优先以 AppImage 形式发布。

发布版本将提供在 [GitHub Releases](https://github.com/xezzon/agent-maestro/releases)。

## 计划中的功能

- [ ] Provider / 模型、MCP、Skills 的统一管理与投影
- [ ] Profile 与 Agent 工具绑定
- [ ] 配置跨设备同步
- [ ] 第三方插件机制
- [ ] 更多 Agent 工具适配（Cherry Studio 等）
- [ ] Skills 在线注册表浏览与安装

## 早期尝鲜：从源码运行

发布版本就绪前，早期用户可以从源码运行。前置依赖：[Rust](https://www.rust-lang.org/)（stable）、[Node.js](https://nodejs.org/) 和 [pnpm](https://pnpm.io/)。

```bash
pnpm install
pnpm tauri dev      # 开发模式运行
pnpm tauri build    # 构建安装包
```

## 许可证

[MIT](LICENSE)
