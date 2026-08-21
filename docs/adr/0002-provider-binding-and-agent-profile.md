# Provider/model 与 MCP/Skill 分层：Provider Binding 与 Agent Profile

- Status: accepted

每个 Agent Tool 的配置拆成两层独立管理：**Provider Binding**（引用的 Provider 及 override，每个工具唯一一份、无命名、无 active 切换）和 **Agent Profile**（引用的 MCP 与 Skill 及 override，每个工具可持多份具名配置，仅一份 active）。两者在 UI 上各有独立的 Apply 按钮；Plugin 为 Provider、MCP、Skill 三类内容分别注册 Projection Target，可以分别落在不同文件/目录（不同 Agent Tool 的配置组织方式差异很大，例如 Skill 往往是目录而不是配置文件中的键）。

最初的设计是单一的"Agent Profile"同时承载三类内容并整份切换，但实践上 provider/model 选择是"账号/网关"层的决定，不随"我现在做什么任务"变化；而 MCP/Skill 组合才是真正的"任务上下文"，值得多份命名并频繁切换。把两层绑在一起，会迫使用户为每个任务 context 重复勾选 provider。

后果：Plugin 投影接口按 Target 分别注册而非单一"写配置文件"；存储上每个 Agent Tool 有一个 provider_binding 字段和一个 profiles 数组。如果未来出现"同一工具需要多套 provider 账号快速切换"的强需求，再引入多份 Provider Binding；v1 不做。
