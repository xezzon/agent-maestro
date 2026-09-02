# Agent Maestro

Agent Maestro 是一个跨平台桌面应用，集中管理各 Agent 工具（Pi、OpenCode、Zed……）的配置：LLM Provider、模型、MCP Server、Skills 与 Profile。配置在 Maestro 中录入一次，再投影（projection）到各工具自己的配置文件中；配置以 Maestro 为准。

## 术语

**Provider**：
用户自行配置的一条 LLM API 接入，以用户自定义的**唯一 slug** 标识（slug 同时兼作界面展示名），可对多种协议（protocol）各持一个 Base URL 端点，并携带一份共享的凭证（API Key）与一份共享的模型列表。多条 Provider 可以共享同一个 Base URL（例如同一网关下的不同账号），靠各自不同的 slug 区分；slug 一旦创建不可更改，且仅允许 `[a-z][a-z0-9-_]*` 的字符。
_避免_：vendor、服务商、模型 Provider（会把「一条接入」与「一家公司/预设目录」混为一谈）

**协议（protocol）**：
Provider 与 LLM API 对话所用的线协议，取二选一的枚举：`openai-completions`（OpenAI 兼容的 Chat Completions 接口）或 `anthropic-messages`（Anthropic Messages API）。一个 Provider 可对两种协议各持一个端点（共享同一凭证与模型列表），也可能只配置其中一个；界面通常要求至少填一个。协议决定了端点指向哪类接口，以及将来投影、测试连接时如何构造请求。
_避免_：类型、type、kind（语义模糊）

**端点（endpoint）**：
某个协议下可访问该 Provider 的 Base URL，如 `openai-completions` 协议下的 `https://api.example.com/v1`。一个 Provider 的每个已配置协议至多有一个端点，未配置的协议其端点值为空。
_避免_：base url（作为泛指时）

**Model（模型）**：
某个 Provider 提供的 LLM 能力，以各 Agent 工具实际使用的模型 ID 字符串（如 `gpt-4o`）标识。模型不是全局实体——它从属于且仅从属于一个 Provider；同一个模型 ID 可以出现在多个 Provider 下（例如直连 OpenAI 与公司网关各有一条 `gpt-4o`）。可选的人类可读显示名只是元数据。
_避免_：model id（当指条目本身、而非 ID 字符串时）

## 凭证

**API Key（API 密钥）**：
调用 Provider 的凭证，可有可无——本地网关（如 Ollama）无需凭证。设置后只写入系统密钥链，永不回读给界面；Provider 记录里保存的是密钥引用，而非密钥本身。
_避免_：token、credential（当作 Provider 的专有概念时）

**密钥引用（secret reference）**：
标识系统密钥链中某条密钥的 URI，形如 `secret://io.github.xezzon.agent-maestro/provider/<id>/api_key`。它代替密钥本身出现在配置中，使配置文件不包含任何秘密；值为空串则表示该 Provider 未设置凭证。
_避免_：密钥占位符、secret url
