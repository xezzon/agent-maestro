# Agent Maestro

Agent Maestro 是一个跨平台桌面应用，作为各 Agent 工具（Pi、OpenCode、Zed 等）的统一配置中心：Provider、模型、API Key 等在此集中维护，再投影到各工具。

## Language

**Tool（Agent 工具）**:
一个被 Maestro 适配的外部 Agent CLI/编辑器（如 Pi、OpenCode、Zed），Maestro 把配置写入它自己的配置文件供其消费。
_Avoid_: target、目标工具、应用

**Provider**:
一个模型服务来源，由 base URL、认证凭据和一组可用模型构成。Provider 可同时暴露多种协议的端点（如 OpenAI 兼容、Anthropic 兼容）。Provider 是 Maestro 侧的统一配置单元。
_Avoid_: 供应商、服务方

**Model**:
Provider 提供的一个具体模型，有在 Maestro 内使用的 id 和给用户看的展示名。

**Plugin（插件）**:
把 Maestro 的统一配置投影到某个特定 Agent 工具的适配单元。插件遵循 Maestro 的插件 SDK，由宿主动态加载；内置插件与第三方插件一视同仁。每个插件声明自己适配的工具；一个工具可被多个插件适配。
_Avoid_: adapter、适配器、driver

**Projection（投影）**:
Maestro 把其持有的统一配置写入某个 Agent 工具配置文件的动作。Maestro 是配置的唯一事实来源，投影会覆盖工具内的手动改动。
_Avoid_: sync、同步、写入、apply

**Secret（凭据）**:
一个认证凭据（如 Provider 的 API Key）。Secret 在 Maestro 的配置中只以引用（secret_ref）出现；投影到各工具时，按该工具自身的存储方式落地，不再受 Maestro 保护。
_Avoid_: API key、token、明文密钥
