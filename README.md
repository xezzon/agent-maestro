# Agent Maestro

> Agent 工具的统一配置中心：Provider、模型、MCP、Skills，配置一次，处处生效。

Agent Maestro 是一个跨平台桌面应用，用于统一管理 [Pi](https://pi.dev)、[OpenCode](https://opencode.ai)、[Zed](https://zed.dev) 等 Agent 工具的配置。Provider、模型、MCP Server 和 Skills 在 Maestro 中集中配置后，通过投影（projection）写入各工具的配置文件，无需在各工具中分别配置。

> ⚠️ **项目状态：早期开发中。** 当前还没有发布版本，功能仍在实现中。

## 功能

- **统一管理 Provider 和模型**：集中维护模型 Provider、模型与 API Key，并投影到各 Agent 工具。一个 Provider 可对 `openai-completions` 与 `anthropic-messages` 两种协议各持一个 Base URL 端点，投影时按所选协议取用；Provider 可逐条启用或禁用，禁用后不参与投影。API Key 以明文保存在本地配置文件 `~/.maestro/config.json` 中，**仅存储在本地，不会上传到任何服务器**。
- **全局变量与插值**：在全局配置中定义具名变量并给出默认值，配置值里用占位符 `${NAME}`、`$NAME`、`${NAME:default}` 引用；投影时由 Maestro 完成一次插值，插件只看到替换后的字面值。
- **插件化适配 Agent 工具**：每类工具由一个 WASM 组件插件适配；内置 Pi 插件随应用分发，其余插件可经 https 来源（正式发布渠道）或本机 file 来源（本地调试）安装、更新、启停与移除。
- **投影写入工具配置**：把 Maestro 中的配置写入各工具自己的配置文件，是「配置一次、处处生效」的落地环节。配置以 Maestro 为准。
- **配置跨设备同步**：在多台电脑之间同步 Maestro 的配置（规划中，尚未实现）。

## 支持的 Agent 工具

- [x] [Pi](https://pi.dev)——内置插件
- [x] [OpenCode](https://opencode.ai)——由 [agent-maestro-plugins](https://github.com/xezzon/agent-maestro-plugins) 提供插件
- [x] [Kimi Code](https://github.com/MoonshotAI/kimi-code)——由 [agent-maestro-plugins](https://github.com/xezzon/agent-maestro-plugins) 提供插件
- [ ] ...

## 使用流程

1. **添加 Provider**：填入协议的 Base URL 端点、API Key 与可用模型；两个协议都配置时必须选定投影使用的协议。当前不内置 Provider 目录，需手动添加所使用的 Provider。
2. **维护全局变量**（可选）：定义变量并在配置值中用占位符引用，投影时自动替换为变量值。
3. **添加插件**：内置 Pi 插件开箱即用；OpenCode、Kimi Code 等工具填入其 Release 上 `manifest.json` 的 https 地址即可安装（插件由 [agent-maestro-plugins](https://github.com/xezzon/agent-maestro-plugins) 发布），本地开发的插件则以指向本机 manifest.json 的路径添加。
4. **执行投影**：在 Provider 页点击「应用到工具」，Maestro 调用所有已启用且加载成功的插件，把配置写入各工具的配置文件，之后即可在对应的 Agent 工具中使用。

## 使用须知

- **配置以 Maestro 为准**：请始终在 Maestro 中修改配置后重新投影。直接在 Agent 工具里手动改动的配置不会被 Maestro 读取，下次投影时可能被覆盖。
- **日志仅存本地**：运行日志写入 `~/.maestro/logs/agent-maestro.log`（按 1 MiB 轮转，保留 15 份归档，总占用约 16 MiB 封顶），报障时可直接提供该目录下的文件；日志中绝不包含 API Key。需要更详细的日志时，以环境变量 `MAESTRO_LOG_LEVEL`（`error|warn|info|debug|trace`，默认 `info`）启动应用。
- 对各 Agent 工具的适配由插件（Plugin）提供：内置 Pi 插件；OpenCode、Kimi Code 插件见 [agent-maestro-plugins](https://github.com/xezzon/agent-maestro-plugins)。

## 插件开发

第三方插件以 WASM 组件实现，经 https 来源（正式发布）或 file 来源（指向本机 manifest.json，本地调试）安装。面向插件作者的完整文档——类型化绑定与依赖写法、manifest 字段语义、https 发布流约定、本地调试回路、`config_dir` 的声明语法与安全规则、沙箱边界——见 [crates/maestro-plugin-sdk/README.md](crates/maestro-plugin-sdk/README.md)。官方维护的第三方插件集合（OpenCode、Kimi Code）见 [agent-maestro-plugins](https://github.com/xezzon/agent-maestro-plugins)，可作范例。

## 支持的平台

基于 Tauri v2，Maestro 提供桌面端应用，支持以下平台：

- **Windows**: Windows 10 及以上（x86_64）。提供安装包（NSIS）。
- **macOS**: Intel 与 Apple Silicon（aarch64）。提供 `.app` / `.dmg`。
- **Linux**: x86_64、aarch64。优先以 AppImage 形式发布。

发布版本将提供在 [GitHub Releases](https://github.com/xezzon/agent-maestro/releases)。

## 计划中的功能

- [ ] 每个插件的配置与变量覆盖（插件的额外配置文件）
- [ ] MCP Server 的录入与投影
- [ ] 配置跨设备同步（WebDAV / Git）
- [ ] 将 API Key 保存到系统密钥链
- [ ] 维护 Provider 数据库并在线获取模型列表
- [ ] 更多 Agent 工具适配（Zed、Claude Code、DeepSeek Harness 等）
- [ ] 以 CLI 的方式使用 agent-maestro
- [ ] 应用自动升级、界面国际化（中文 / 英文）与主题设置

## 早期尝鲜：从源码运行

发布版本就绪前，早期用户可以从源码运行。前置依赖：[Rust](https://www.rust-lang.org/)（stable）、[Node.js](https://nodejs.org/) 和 [pnpm](https://pnpm.io/)。

```bash
pnpm install
pnpm tauri dev      # 开发模式运行
pnpm tauri build    # 构建安装包
```

## 许可证

[MIT](LICENSE)
