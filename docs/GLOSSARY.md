# Agent Maestro

Agent Maestro 是一个跨平台桌面应用，集中管理各 harness（Pi、OpenCode、Zed……）的配置：LLM Provider、模型、MCP Server、Skills 与 Profile。配置在 Maestro 中录入一次，再投影（projection）到各工具自己的配置文件中；配置以 Maestro 为准。

## 术语

**Provider**：
用户自行配置的一条 LLM API 接入，以用户自定义的**唯一 slug** 标识（slug 同时兼作界面展示名），可对多种协议（protocol）各持一个 Base URL 端点，并携带一份共享的凭证（API Key）、一份共享的模型列表与一份共享的自定义 HTTP header（`custom_header`），记录投影实际使用哪一个端点（未记录或记录失效时由插件兜底，见 ADR 0016）。多条 Provider 可以共享同一个 Base URL（例如同一网关下的不同账号），靠各自不同的 slug 区分；slug 一旦创建不可更改，且仅允许 `[a-z][a-z0-9_-]*` 的字符。Provider 有一个启用状态（默认启用，旧配置缺此字段视为启用）：禁用后不参与投影，已投影到工具侧的条目随之消失；禁用态仍可编辑。启用状态只影响「是否参与投影」，不改变落盘一致性——禁用 Provider 的占位符仍须对变量表可解析，存储写出的每条记录都自洽。
_避免_：vendor、服务商、模型 Provider（会把「一条接入」与「一家公司/预设目录」混为一谈）

**协议（protocol）**：
Provider 与 LLM API 对话所用的线协议，取二选一的枚举：`openai-completions`（OpenAI 兼容的 Chat Completions 接口）或 `anthropic-messages`（Anthropic Messages API）。一个 Provider 可对两种协议各持一个端点（共享同一凭证与模型列表），也可能只配置其中一个；界面要求至少填一个，两个都配置时必须选定投影使用的协议。协议决定了端点指向哪类接口，以及将来投影、测试连接时如何构造请求。
_避免_：类型、type、kind（语义模糊）

**端点（endpoint）**：
某个协议下可访问该 Provider 的 Base URL，如 `openai-completions` 协议下的 `https://api.example.com/v1`。一个 Provider 的每个已配置协议至多有一个端点，未配置的协议其端点值为空。投影实际用哪个端点由插件裁定（选择有效→用它，否则唯一端点，多端点且无有效选择→优先 `openai-completions`，见 ADR 0016；工具只支持一种线协议时，协议由该工具本身决定，插件直接取该协议端点，见 ADR 0021）；宿主只把全部端点与选择原样交给插件，零端点的 Provider 由宿主在投影前跳过。
_避免_：base url（作为泛指时）

**Model（模型）**：
某个 Provider 提供的 LLM 能力，以各 harness 实际使用的模型 ID 字符串（如 `gpt-4o`）标识。模型不是全局实体——它从属于且仅从属于一个 Provider；同一个模型 ID 可以出现在多个 Provider 下（例如直连 OpenAI 与公司网关各有一条 `gpt-4o`）。可选的人类可读显示名只是元数据；模型还可声明自己的 token 能力上限（`limit`：上下文窗口、单次最大输入、最大输出）与能力集合（`capabilities`：`tool_use`、`image_in`、`thinking`）——这些是**声明式**元数据，Maestro 只保存与投影，不据此推断或裁剪请求。
_避免_：model id（当指条目本身、而非 ID 字符串时）

**harness**：
被 Maestro 统一管理配置的 Agent 软件（Pi、OpenCode、Zed……）。每类 harness 由一个**插件**适配，其配置文件由插件在投影时写入。
_避免_：Agent 工具、Agent Tool、客户端、应用（会与 Maestro 自身混淆）

**投影（projection）**：
把 Maestro 中录入的配置写入各工具自身配置文件的动作，是“配置一次、处处生效”的落地环节。配置以 Maestro 为准：工具侧的手工改动不被读取，投影时可能被覆盖。
_避免_：应用（作动词时）、同步（专指跨设备同步）

## 变量与插值

**变量（variable）**：
全局配置中一个具名的可替换值。它在全局配置里定义并自带默认值；每个插件都可以在自己的插件配置中覆盖它的值，未覆盖则用默认值。
_避免_：参数、常量、环境变量

**占位符（placeholder）**：
配置值中引用变量的标记；标记以外的一字一符都原样保留。
_避免_：变量标记、token

**插值（interpolation）**：
把配置值里的占位符替换为变量实际值的过程。插值在投影时由 Maestro 执行一次，插件只看到替换后的字面值。
_避免_：模板、渲染

## 凭证

**API Key（API 密钥）**：
调用 Provider 的凭证，可有可无——本地网关（如 Ollama）无需凭证。真值只保存在**系统密钥链**；Provider 的 `api_key` 字段只保存空串（未设置）或**密钥引用**（已设置），真值永不落进配置文件。界面只显示「已设置 / 未设置」，永不回读真值；写入只有「不变 / 更换 / 清除」三种意图。迁移失败时字段可暂时保留旧明文，界面标记为「明文待迁移」，下次启动重试。
_避免_：token、credential（当作 Provider 的专有概念时）

**密钥链（keychain）**：
操作系统提供的凭证存储（macOS Keychain、Windows Credential Manager、Linux Secret Service），是 API Key 真值的唯一持久落点。Maestro 以命名空间 `io.github.xezzon.agent-maestro`、account `provider/<slug>/api_key` 存放每个 Provider 的凭证；密钥链不可用时如实报错，绝不回退为明文。
_避免_：密码库、凭据管理器（作为泛指时）

**密钥引用（secret reference）**：
配置文件里指向密钥链条目的 URI：`secret://io.github.xezzon.agent-maestro/provider/<slug>/api_key`。它不含秘密，可随配置复制；引用与明文在类型上可区分，`secret://` 是保留前缀，明文密钥不能以它开头。引用与 slug 绑定（slug 不可变，引用因此稳定）；引用不合法即整份配置进入保护态。
_避免_：链接、指针

## 插件

**插件（Plugin）**：
适配一类**工具**、把 Maestro 配置投影进该工具配置目录的可安装组件。插件以 manifest 描述自身（`id`、写入目录与入口 wasm 的回源地址），经由**来源**获取；它适配哪类工具由 `id` 表明（插件作者应把 id 命名得足以看出是哪类工具），写入目录以平台变量前缀声明、由宿主按当前平台解析为具体路径。
_避免_：扩展、适配器

**内置插件（builtin plugin）**：
随应用一同分发的插件，wasm 内嵌于二进制；安装上与其余来源同一管线（启动时落位并写配置条目）。可禁用，不可移除。
_避免_：官方插件（官方经 https 来源发布的插件不是内置插件）

**来源（source）**：
插件条目的获取途径，同时是配置中插件条目的唯一标识：内置于应用（`builtin:<id>`）、一个指向 manifest.json 的 https URL（正式发布渠道，仅接受 https），或一个指向 manifest.json 的本机绝对路径（本地调试，视同开发者模式）。不同来源解析出相同的插件 id 视为冲突。
_避免_：仓库（仅描述插件作者的开发仓库时）、Git 地址（来源模型已废弃 Git）、http 来源（来源 URL 仅接受 https，不称 http）

**插件配置（plugin config）**：
某个插件独有的配置，独立于全局配置：它覆盖全局**变量**的值，并承载该插件私有的**表单**。它与插件一一对应，随插件存在。
_避免_：额外配置文件、局部配置

**表单（form）**：
插件私有的一组配置项，形状由插件自己在 manifest 中声明。Maestro 只按声明渲染成表单、把内容原样转交，不解释其含义。
_避免_：设置、settings、选项

## 偏好与外观

**偏好（preference）**：
应用级的用户偏好，记录「应用本身应当是什么样」的选择，独立于 Provider、变量、插件等领域配置，不参与投影。偏好随应用启动加载，变更即时生效。
_避免_：设置、settings、选项

**主题（theme）**：
应用的明暗外观，取三选一：亮色、暗色、跟随系统；选「跟随系统」时随操作系统的明暗外观变化。
_避免_：皮肤、配色方案
