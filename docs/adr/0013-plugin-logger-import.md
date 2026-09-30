# 0013: 插件日志经 logger import 落应用日志，WIT 合同升级至 1.1.0

插件投影过程存在需要外显的非致命事件：逐条容错场景（单条处理失败后跳过、继续投影其余条目）的消息在 `write-providers` 的「成功返回文件列表 / 失败返回单个错误字符串」两种结果里无处安放，只能静默丢弃。ADR 0008 落地日志设施时把「WASM guest 日志通道」明确推迟单开（即 #82）。我们决定在 `maestro:plugin` 1.1.0 里新增 `logger` 接口（`level` 枚举 error/warning/info/debug + `log: func(level, message)`），在 `plugin-world` 里声明为 import，宿主 linker 恒定提供——组件不引用该 import 即不受影响。SDK（同步升 1.1.0）把 `log`/`Level` re-export 到 crate 根，插件一行引入；消息由插件自行拼装，形状刻意最小（无结构化字段、无 trace），后续扩展遵循 WIT 版本约定另行升级。

去向只有应用日志：宿主把每条消息按 level 映射为 `log` 门面的级别（error→ERROR、warning→WARN、info→INFO、debug→DEBUG）写入应用日志，行前缀携带插件 id（import 调用方无法自证归属，由宿主从 manifest 补上）。debug 默认不落盘由全局级别旋钮（ADR 0008，默认 INFO）承担，`MAESTRO_LOG_LEVEL=debug` 时展开，不引入第二把级别旋钮。实施中决策收缩：issue 初版设想把消息收集进投影结果、按插件归属在 UI 呈现，落地时改为只落应用日志——投影结果的受众是「这次投影做了什么」的用户，过程性消息的受众是排障的开发者，两者混排只会给投影弹窗加噪音；且落应用日志后消息天然带上时间线与轮转归档，证据属性更强。插件不调用 `log` 时行为与现状完全一致，UI 零改动。

兼容取舍是本 ADR 对既有约定的修订：SDK 文档曾承诺「WIT 包版本升级时宿主同时注册新旧接口版本，给插件作者渐进迁移窗口」，本 ADR 收回该承诺——宿主只注册当前 WIT 包版本的导出，1.0.0 → 1.1.0 升级后按旧合同编译的组件必须在宿主侧链接失败、以新版 SDK 重编译。决定性理由：合同导出名逐字含包版本（`maestro:plugin/plugin@1.0.0` vs `@1.1.0`），双版本注册意味着双份 bindgen、实例化期版本探测与两套绑定类型间的逐字段转换，机器成本刚性；而当前生态只有内置 pi，「渐进迁移窗口」没有真实需求方。`log` import 本身对此无影响——它是宿主侧恒定提供的 import，老组件不引用即不受影响，但它随包版本升级落地，等于一次「只加不改」的 minor 升级照常要求重编译。SDK 的版本对齐约定（SDK 1.x ↔ WIT 1.x）不变。

**message 中不得携带凭证**是硬约束：宿主不做内容审查（与 ADR 0008 的信任模型一致——插件本就持有 api_key，宿主过滤既不可靠又造误报），约束落在 SDK 文档（`crates/maestro-plugin-sdk/README.md`，兼作插件作者指南）；插件把秘密上报进日志文件属于与「插件在错误消息里回显密钥」同类的已知残余风险。验证只覆盖级别映射：它是纯函数，单测钉住 error/warning/info/debug 四个分支。import 的宿主接线（linker 注册、实例化与投影不受影响）没有自动化测试——实施中曾构建验证夹具 `log-fixture`（按四个 level 各上报一条后返回空文件列表，随宿主构建自动编译），评审后按「不值得为一个测试维护一个插件 crate」的取舍裁掉，接受接线回归的风险，首个引用 logger 的插件接入时以人工验收补上：默认级别下日志文件出现 error/warning/info 三条，`MAESTRO_LOG_LEVEL=debug` 下四条。

曾考虑并否决的形态：收集进投影结果按插件分组展示（见上，决策收缩）；复用 `wasi:logging`（多一套 import 集合与级别体系，和 ADR 0008 的单旋钮打架，wasi 标准库的 logging 包在 WASI 0.2 里也尚未定稿）；把 guest stdout/stderr 接进日志（ADR 0008 已否决——改变 guest 行为、WIT `log` 的版本成本更可控）；宿主按正则审查 message 疑似密钥（不可靠且误报，宁守作者约定）；双版本注册（见上，承诺收回）。
