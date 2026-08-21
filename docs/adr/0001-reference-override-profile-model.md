# Agent Profile 以引用 + override patch 关联 Global 条目

- Status: accepted

Global Profile 中的条目（Provider/MCP/Skill）是具名、可复用的目录；Agent Profile 与 Provider Binding 只持有对这些条目的**引用**和一份字段级 override patch，不复制条目值。Global 条目改动时，所有引用方的 effective 值即时变化；override 按"标量替换、map deep merge、list 整体替换"规则与 Global 合并，不能表达删除某个 Global key。

引用不拷贝，代价是"引用目标消失"的场景必须定义。采用**硬删除 + 悬空引用（dangling reference）**：Global 条目可被直接物理删除（slug 释放、可复用），删除不阻止、不校验引用；仍引用它的 Provider Binding 或 Agent Profile 保留该引用与 override，成为悬空引用。执行 Apply 时，Maestro 在投影前列出全部悬空引用并警告，然后跳过这些条目继续投影其余内容；引用是否存活只由"目标 slug 是否存在"决定——若日后创建同 slug 的新条目，悬空引用自动恢复生效（Maestro 不做新旧条目区分）。

不采用"值拷贝 + 同步"：拷贝模型需要记录基线、处理字段级冲突、提供"同步"按钮，而配置场景的真实期望是"改全局，所有 agent 立刻看到"。早期版本曾选择"软删除 + broken 状态 + 还原"，后经复核放弃：为"可恢复"维护的标记/还原/purge 状态机与收益不成比例，误删可由 Apply 警告与"同 slug 自愈"覆盖；也坚持不采用级联删除——它会静默丢弃引用方精心配置的 override（尤其是 API key 等手输字段）。悬空引用的 override 在引用被移除前始终保留。
