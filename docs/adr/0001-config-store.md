# 0001: 配置存储于 `~/.maestro/config.json` 单一 JSON 文档

Maestro 需要持久化 Provider、模型等非密钥配置。我们决定把所有配置放在用户主目录下的固定路径 `~/.maestro/config.json`，作为单一 JSON 文档，由 Rust 侧统一读写（启动时加载进内存、变更后原子写入），不引入数据库。

选择 `~/.maestro` 而非 Tauri 默认的应用配置目录（macOS `~/Library/Application Support`、Linux `~/.config`、Windows `%APPDATA%`），是因为：主目录下的点目录与 Zed、OpenCode 等兄弟 Agent 工具各在主目录落配置的惯例一致，用户可见、易备份、跨平台路径统一。选择单一 JSON 文档而非 SQLite：数据形态是小型嵌套文档，JSON 序列化零成本；将来做跨设备同步时单文件可整体比较/合并，SQLite 的跨设备合并是公认痛点；用户可自行编辑备份。代价是偏离 Tauri 惯例、文件需自行处理原子写与损坏保护（见决策 20：坏文件拒绝写入，绝不静默覆盖）。
