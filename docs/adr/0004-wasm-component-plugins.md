# 0004: 插件以 WASM 组件实现，直接写入工具配置目录

各 Agent 工具的配置格式异构（JSON、TOML 乃至 SQLite），把 Maestro 配置投影进工具配置文件的能力交给可插拔的插件承载：内置插件随应用分发，第三方插件从 Git 仓库安装（来源模型后被 ADR 0006 改为 https URL 与本地路径，Git 来源废弃）。我们决定插件是 WASM 组件（wasmtime 宿主，Component Model / WASI 0.2），接口为 WIT 合同 `maestro:plugin`（v1 仅导出 `write-providers`：输入类型化的 provider 列表——每条只带一个协议的端点——返回已写入文件路径或错误字符串）；插件 metadata 只有唯一来源，即插件根目录的 manifest.json（`id`/`name`/`tool`/`config_dir`/`entry`，容忍未知字段，必填缺失即加载失败），宿主直接读文件，组件不导出 get-metadata。

写入与装载模型：宿主不代写文件，而是把 manifest 声明的 `config_dir`（`~` 展开，目录不存在则先创建）预开放给组件，插件在沙箱内经 WASI 文件系统直接落盘——这是插件唯一被授权的写入面，也因此 TOML、SQLite 等"非纯文件"配置天然可支持；代价是宿主对写入内容零可见（第一期不做写入预览）。git 来源（git2，匿名 HTTPS、默认分支、repo 根即插件根）先克隆到临时目录、读 manifest 得 id、再落位 `~/.maestro/plugins/<id>`；内置插件以字节内嵌于二进制、不物化到磁盘。配置中的 `plugins` 段（`source` + `enabled`，纯增量字段不 bump version）记录用户意图：添加时先写 config 条目，随后的下载/加载失败保留条目、插件进错误态，可经"更新"（重新克隆，成功才替换旧目录）或"重新加载"（仅读磁盘、不联网）重试；应用启动只读磁盘，离线可用。

曾考虑并否决的形态：前端 JS 模块（插件与应用同权限，无进程级沙箱）；子进程可执行插件（跨平台的解释器与执行位依赖是硬伤）；声明式模板插件（表达不了 SQLite 类变更，且会催生迷你模板语言）；宿主代写文件的写入计划（`path + content` 同样表达不了数据库变更）。类型化 WIT 合同把 Provider 领域模型焊进接口版本：增改字段即 breaking change，需升级 WIT 包版本并要求插件重编译——接受此代价，换取插件作者获得原生类型绑定。git 插件即"用户信任并安装的代码"，与自行安装 npm 包同级：沙箱隔离能力滥用，不审查内容。
