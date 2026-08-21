# Context

本文件是 Agent Maestro 的领域语言表（glossary）。仅记录术语与关系，不记录实现决策。

## 术语

### Agent Tool

被 Maestro 管理的外部 Agent 应用（如 Zed、OpenCode、Pi Agent）。每个 Agent Tool 拥有自己的配置文件格式与文件位置。

### Plugin

Maestro 与某个具体 Agent Tool 之间的适配器。Plugin 声明它为该 Agent Tool 处理哪些**投影目标（Projection Target）**，并负责把对应层的配置渲染并写入目标位置。单向：不从工具侧读取或回写。

每个 Plugin 可以注册三类目标：

- **Provider Target**：写 provider/model 配置（某些工具是配置文件的一段，某些工具是独立文件）。
- **MCP Target**：写 MCP server 配置（可能与 provider 不在同一个文件）。
- **Skill Target**：把 Skill 目录部署到工具期望的位置（可能是软链、拷贝、或写入搜索路径配置）。

一个 Plugin 可以只实现其中一部分（例如某工具不支持 Skill，则不注册 Skill Target）。

### Global Profile

Maestro 维护的**单例**中心配置库（catalog）。没有名字、不可切换、不存在 active/inactive 之分——它就是 Maestro 内部唯一的全局目录。若单独说“Profile”而不限定 Global，默认指 Agent Profile。

Global Profile 包含三组条目：

- **Provider**：模型服务的接入定义（含 base URL、API key、可用 model 列表等）。一个 Provider 条目中包含该 provider 下的全部可用 model（字符串列表），model 本身不是独立条目、不独立于 Provider 存在。
- **MCP**：MCP server 的接入定义。采用**判别联合（tagged union）**按 transport 区分字段：
  - `type: "stdio"` → `{ command, args?, env?, cwd? }`；
  - `type: "http"` → `{ url, headers? }`（对应 MCP 规范的 Streamable HTTP 传输）。

  v1 仅支持上述两种 transport；legacy SSE 不纳入。OAuth 等授权流程不由 Maestro 实现或代持——Maestro 只投影连接信息（如 URL、headers），令牌的获取与刷新由目标 Agent Tool 在运行时自行完成。
- **Skill**：一个可复用的 Agent Skill 条目，与 [Agent Skills Specification](https://agentskills.io/specification) 对齐——即一个包含 `SKILL.md`（Markdown，含 frontmatter 元数据）及其附带资源（脚本、模板等）的目录。Skill 条目录入该目录的**来源**：v1 支持本地路径（`source.kind = "local"`），不内联 prompt 文本、不让 Maestro 代持 Skill 的正文内容。Skill 目录如何暴露给目标 Agent Tool（软链、拷贝、写入搜索路径等）由对应 Plugin 决定。

Global Profile 中的每个条目都有一个在同组内唯一的 **slug** 作为稳定标识：

- 创建时由用户指定，字符集 `[a-z0-9-_]`，长度 1–64；
- 在同组（Provider / MCP / Skill）内唯一，跨组可重名；
- **创建后不可变**——“改名”通过新建条目 + 迁移各 Agent Profile 引用 + 删除旧条目完成。

slug 同时用于：(a) Agent Profile selection 的引用目标；(b) Plugin 投影时派生目标配置文件中的子键名。

### Provider Binding

针对某个 Agent Tool 的 **provider/model 层配置**。每个 Agent Tool 持有且仅持有一份 Provider Binding——无 slug、无 active 切换。它包含：

1. **Selection**：从 Global Profile 引用的一组 Provider 条目。粒度是整个 Provider 条目：一旦引用，该 Provider 下的所有 model 即在该 Agent Tool 中可用，不做 per-model 启停。可同时引用多个 Provider。不记录默认 model（默认 model 是 Agent Tool 自己的交互状态）。
2. **Overrides**：对所引用 Provider 条目的字段级遮蔽（patch），例如 API_KEY、base_url、headers。合并规则与下文 Agent Profile Overrides 一致。

### Agent Profile

针对某个 Agent Tool 的 **MCP + Skill 层配置快照**。每个 Agent Tool 可以持有多份**具名** Agent Profile，但同一时刻只有一份处于 **active** 状态。

- **Active 切换即投影**：将某份 Agent Profile 设为 active 时，Maestro 立即对该 Profile 的 MCP 层与 Skill 层执行一次 Projection（等价于一次 Apply Agent Profile）。切换本身是显式意图，不投影 Provider Binding。旧 active 遗留在 MCP/Skill Target 上的 Maestro 拥有项按对账规则清除。
- **内容改动不自动投影**：对 Profile 内容（selection、overrides）、Provider Binding、Global 条目的编辑都不自动写盘，需用户显式 Apply 才会落盘。

一份 Agent Profile 由两部分组成：

1. **Selection**：从 Global Profile 中**引用（reference）**的 MCP、Skill 条目集合。这里是引用，不是拷贝——Global 条目的值变化时，所有引用它的 Agent Profile 即时生效，无需显式同步动作。
2. **Overrides**：对所引用条目的部分字段进行**字段级遮蔽（patch）**，例如 MCP server 的 env、headers。解析时按下列规则与 Global 条目合并（Provider Binding 与 Agent Profile 共用同一算法，所有 Plugin 共享）：
   - **标量**（string/number/bool）：override 替换 Global 值；
   - **map**（object，如 `env`、`headers`）：递归 deep merge，key 级替换；
   - **list/array**（如 `args`）：整体替换，不拼接。

   Override 不能表达“删除”某个 Global key；如果某个 key 不应出现在任何 Agent Profile 中，必须直接修改 Global Profile。

Global Profile 中的条目只能被**硬删除（hard delete）**：条目被物理移除，其 slug 随之释放并可被新条目复用。删除不校验引用——仍引用该条目的 Provider Binding 或 Agent Profile 留下**悬空引用（dangling reference）**：引用与其 override 原样保留，但指向一个不存在的目标。

悬空引用不阻止其它条目的投影。Maestro 在 UI 中对悬空引用做轻量标记（引用方显示 ⚠ 与 tooltip，标明哪个 slug 已不存在），该标记从数据派生（slug 在 Global 中不存在即为悬空），不引入额外状态。执行 Apply 时，Maestro 在投影前弹**确认对话框**列出全部悬空引用，由用户选择“继续 Apply（跳过这些条目）”或“取消”；随后跳过这些条目继续投影其余内容。引用是否存活只由目标 slug 是否存在决定；若日后创建同 slug 的新条目，悬空引用自动恢复生效（Maestro 不做新旧条目区分）。

### Projection Target

Plugin 为某个 Agent Tool 声明的一个写入目的地。每个 Projection Target 有自己的类型与位置，例如：

- 一个配置文件中的若干**键路径**（如 `settings.json` 里的 `language_models`）；
- 一个独立文件（如某工具专门的 `mcp.json`）；
- 一个目录（如 Skill 目录，每个 Skill 是其中一个子目录）。

Plugin 为三类内容分别声明 Projection Target：**Provider 层**、**MCP 层**、**Skill 层**。三者可以指向同一个文件的不同键路径，也可以指向完全不同的文件/目录——由目标 Agent Tool 自身的配置组织决定。

### Projection

把某一层的 effective 值渲染并写入对应 Projection Target 的过程。单向。Projection 由对应 Plugin 执行，**按层独立触发**：

- Provider Binding 的 Apply 只投影 Provider 层；
- Agent Profile 的 Apply（或切换 active）只投影 MCP 层与 Skill 层。

对于“文件 + 键路径”型 Target，写入边界遵循两条规则：

1. **路径级接管**：Plugin 声明该 Target 接管文件中的哪些键路径；被接管路径之外的文件内容原样保留，Maestro 永不触碰。
2. **按条目对账（reconcile by entry）**：在每个被接管路径下，Maestro 以本次 Projection 结果为准。Maestro 拥有的子键由所引用 Global 条目的稳定 slug 派生；结果中不存在的 Maestro 子键被删除，文件中不属于任何 Maestro 条目的未知子键原样保留。

对于“目录”型 Target（如 Skill 目录），对账粒度是子目录/文件：Maestro 管理的条目对应目录中同名子项，不在结果中的 Maestro 子项被删除，其余保留。

换言之：同名项覆盖、缺失项新增、Maestro 不再管理的项删除、其余项不动。

投影由用户**显式触发（Apply）**：Maestro 不自动写入、不维护 desired/last-applied 状态、不做 diff 预览、不检测外部冲突——Apply 即读取当前目标内容按上述边界直接写回。Global 改动通过引用机制即时反映到 effective 值，但不自动落盘。切换 Agent Profile 的 active 等价于对 MCP 层与 Skill 层执行一次 Apply。
