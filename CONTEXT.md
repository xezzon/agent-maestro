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

**工具（tool）**：
被 Maestro 统一管理配置的 Agent 软件（Pi、OpenCode、Zed……）。每类工具由一个**插件**适配，其配置文件由插件在投影时写入。
_避免_：客户端、应用（会与 Maestro 自身混淆）

**投影（projection）**：
把 Maestro 中录入的配置写入各工具自身配置文件的动作，是“配置一次、处处生效”的落地环节。配置以 Maestro 为准：工具侧的手工改动不被读取，投影时可能被覆盖。
_避免_：应用（作动词时）、同步（专指跨设备同步）

## 凭证

**API Key（API 密钥）**：
调用 Provider 的凭证，可有可无——本地网关（如 Ollama）无需凭证。以明文随配置一起保存在本地配置文件中，界面可查看与编辑。
_避免_：token、credential（当作 Provider 的专有概念时）

## 插件

**插件（Plugin）**：
适配一类**工具**、把 Maestro 配置投影进该工具配置目录的可安装组件。插件以 manifest 描述自身（id、适配的工具、写入目录），经由**来源**获取。
_避免_：扩展、适配器

**内置插件（builtin plugin）**：
随应用一同分发的插件，不经下载与安装；可禁用，不可移除，也不出现在添加流程中。
_避免_：官方插件（官方经 https 来源发布的插件不是内置插件）

**来源（source）**：
插件条目的获取途径，同时是配置中插件条目的唯一标识：内置于应用（`builtin:<id>`）、一个指向 manifest.json 的 https URL（正式发布渠道，仅接受 https），或一个指向 manifest.json 的本机绝对路径（本地调试，视同开发者模式）。不同来源解析出相同的插件 id 视为冲突。
_避免_：仓库（仅描述插件作者的开发仓库时）、Git 地址（来源模型已废弃 Git）、http 来源（来源 URL 仅接受 https，不称 http）
