# 0007: maestro-plugin-sdk 以发布 tag 的 git 依赖分发，不发布 crates.io

`maestro-plugin-sdk`（见 ADR 0004 的 WIT 合同 `maestro:plugin`）原方案是在 release 时发布 crates.io，并以 SDK 版本与 WIT 包版本的 semver 对齐（SDK 1.x ↔ WIT 1.x）作为消费端的版本锚点。现决定不发布：crate 元数据置 `publish = false`，release 流程移除发布 job，插件作者以 git 依赖引用本仓库中的 `crates/maestro-plugin-sdk`，并锁定 Maestro 的发布 tag。理由：WIT 合同随 crate 分发，SDK 与宿主必须配套，消费端真正需要回答的是「与哪个宿主版本兼容」；宿主发布 tag 同时锚定宿主版本、SDK 与 WIT 合同，一个 tag 即固化插件与宿主的兼容组合，SDK 自身的 semver 降为内部约定、不再充当消费端解析依据。crates.io 链路（`CARGO_REGISTRY_TOKEN`、发布 job、已发布跳过逻辑）换来的版本化解析，在这里收益不抵成本；插件作者本就面向本仓库生态开发，构建期能访问 GitHub 即可，且 SDK 只在插件编译期使用、不进入宿主运行时。

分发细节：文档给出的依赖写法为 `{ git = "https://github.com/xezzon/agent-maestro", tag = "<发布 tag>" }`，tag 按宿主版本号（`tauri.conf.json` 的 `version`）走；SDK 落地后的首个 release 才有可用的 tag（`v0.1.0` 早于 SDK crate，引用不到）。代价：git 拉取的是整个仓库（含 Tauri 应用），比 crates.io 单 crate 大（仓库尚小，可接受）；`Cargo.lock` 会把解析到的 commit 钉住，构建可复现。WIT 合同仍是单一来源、随 crate 分发，`build.rs` 构建时内嵌生成绑定。

曾考虑并否决的形态：继续发布 crates.io（原方案——需要维护 SDK↔WIT 的 semver 对齐与整条发布链路，且消费端仍要另行确认宿主兼容版本，对齐本身不带来消费端收益）；为 SDK 单独打 tag（如 `maestro-plugin-sdk-v1.0.0`——多一套 tag 与发布流程，且该 tag 只表达 SDK 版本、不表达宿主兼容性，而宿主 tag 已含 SDK）；引用默认分支或 `rev`（分支随 main 漂移、rev 不可读，都不如发布 tag 稳定且能对应到宿主版本）；把 SDK 拆入独立仓库分发（WIT 合同与宿主 `bindgen!` 必须同源，拆分会把单一来源变成两份需同步的合同）。
