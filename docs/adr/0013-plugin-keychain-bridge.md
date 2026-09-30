# 0013: 凭证经宿主适配器桥接进目标工具的钥匙串（`maestro:plugin` v1.1.0）

部分被管理的工具不把 API Key 写进自己的配置文件，只从系统凭证库读取（macOS Keychain / Windows Credential Manager / Linux Secret Service），条目按该工具自己的索引键存放。插件运行在 wasm32-wasip2 沙箱里、够不到凭证库，于是这类工具的凭证只能由用户在该工具里手工填写——「配置一次、处处生效」恰在最需要自动化的那一步断了。我们决定由宿主把系统凭证库桥接成一个 WIT import：契约包升 `maestro:plugin@1.1.0`，新增 `keychain` 接口，只有两个函数——`write(key, secret)` 与 `delete(key)`；`delete` 对不存在的 key 视为成功（幂等），同 key 重复写入为覆盖。world 只在 import 侧加 `import keychain;`，export 侧不变：宿主 linker 恒定提供该 import，因此老插件无需重编译（wasmtime 的 semver-track 名字查找会把宿主请求的 `maestro:plugin/plugin@1.1.0` 落到组件导出的 `@1.0.0` 上），新插件不调用即不受影响。

桥接面故意只有 `(key, secret)`。条目的其余字段（服务名、用户名、标签）是目标工具的内部约定，由宿主的适配器补齐，插件既不该也无从指定；`key` 是该工具侧索引键（Zed 按 API 端点 URL 精确匹配条目），不是 Maestro 的 slug。

manifest 新增可选字段 `keychain_namespace`，声明插件使用哪个凭证适配器。它既是**权限闸门**（未声明即拒绝调用），也是**跨工具事故的隔离带**（适配器只把条目落成自己那种工具的格式，插件写不出别的工具认识的条目）。它不进入 `key` 本身——给 `key` 加前缀或按插件隔离 keychain service 都会让目标工具查不到条目，那等于静默失效。声明了宿主不认识的 namespace 与未声明一样在**调用时**报错，不在装载时报错：闸门关在调用处，插件的装载与配置文件投影不因一个失效的声明连坐。宿主内置一组适配器、按 namespace 选中，格式知识（平台差异、索引键映射、内部常量）收在宿主一处，工具改版失效时只需发宿主版；本期只交付机制与宿主侧骨架，适配器表为空，具体工具的适配器随对应插件另立 issue 落地（落地时附对真实目标工具的冒烟测试）。（本段「宿主内置一组适配器、按 namespace 选中」与「宿主不认识的 namespace 报错」两处已由文末 2026-09-30 的修订取代。）

代价与已知限制：适配器依赖目标工具的内部实现（条目格式与索引键），属逆向耦合，工具改版可能失效——耦合点收在宿主一处是对此的唯一缓解。凭证明文经 WIT 调用传入宿主，但全程不落磁盘明文（宿主只把它交给 OS 凭证库，日志只记 namespace 与索引键）。插件无状态，Maestro 里删除 Provider 后其在目标工具钥匙串里的条目成为孤儿，不会自动清理：文档提示用户在目标工具里手动重置，宿主做 namespace 级 sweep 留待将来。

曾考虑并否决的形态：**插件自带平台凭证库 SDK**（wasm 侧 keyring，glib/Secret Service、wincred、Security framework）——Linux 需要 D-Bus 与会话总线、Windows 与 macOS 需要各自的系统 API，沙箱里做不到，放开等于拆掉沙箱。**由插件指定条目的全部字段**（service/user/label 经 WIT 传入）——把目标工具的内部格式知识摊给每个插件作者，工具改版时要改的是每个插件，与本决策的取向正好相反。**按插件/namespace 隔离 keychain service 或 key 前缀**——目标工具按自己的 service 名与索引键查条目，隔离即读不到。**namespace 声明在安装期校验**（未知即拒绝安装）——声明值随宿主版本演进，安装期拒绝会让「宿主不认识」在升级后变成装不上的插件，而调用期报错只损失凭证那一条。**在 WIT 里传结构化条目**（记录带 service/user/label）——插件仍无从知道目标工具的格式，只是把一个又大又假的灵活面暴露出去。**宿主侧 sweep 清理孤儿条目**——需要宿主枚举该工具的全部条目并判断归属，本期不做（见上文的已知限制）。

机制侧的验收以单元测试承载：未声明 `keychain_namespace` 时 `write`/`delete` 报错；声明后凭证落进该 namespace 的条目（以内存 mock store 断言条目落在哪个 `(service, account)` 上）；另以一份按 v1.0.0 合同构建的插件产物实测老插件的装载与投影不受影响。端到端验收（目标工具真能读到 key）落到对应工具的 issue。

补充（2026-09-29）：宿主侧的系统凭证库读写基于 keyring-rs——条目经 `keyring-core` 建删，平台凭证库（macOS Keychain / Windows 凭据管理器 / Linux Secret Service）由 `keyring` 门面（v1）按平台装成默认 store：首次调用时安装，已装好就不动，装不上只尝试一次，原因逐条随调用回到插件。拆成两个 crate 而非只用门面，是因为门面在平台 store 装不上时一律拒绝建条目、连调用方自备的 store 也不认，本机没有 Secret Service（CI、headless）就用不了内存 mock store，适配器也就无从测试；`keyring-core` 只认默认 store，「已装好就不动」这条规则于是同时保住真机与测试。桥接面不变；宿主据此提供通用适配器 `KeyringAdapter`（`src-tauri/src/keychain.rs`）：条目按 `(service, account=key)` 定位、`secret` 落秘密槽位——索引键聚合成一份荷载（如一条 JSON 记录）的工具用不上这个直接映射，得自己实现适配器。

修订（2026-09-30）：`keychain_namespace` 的语义收敛为**声明值即条目所在的 namespace**（系统凭证库里的 service 名）：宿主不预置 (工具 → 格式) 表，`write`/`delete` 在调用时按 manifest 的声明值落条目——`service` 取声明值、`account` 取插件的 `key`、秘密取 `secret`。上文「宿主内置一组适配器、按 namespace 选中」与「声明了宿主不认识的 namespace 与未声明一样在调用时报错」两处随之作废：闸门只剩「未声明即拒绝」，宿主不再判定认识与否。代价是隔离带减弱——插件能写进任意 service，跨工具写入的防线从「宿主只认名单上的工具」退回「声明即授权」，声明值认错（多了前缀、认错了工具）不再当场报错，而是目标工具读不到条目这种静默失效，安装期的权限提示因此以声明值为准；换来的是宿主不必为每个工具预写一份格式表，条目就是这个直接映射的工具接入时无需改宿主。条目格式不是这个映射的工具（如整份凭证存成一条 JSON）仍要另立适配器。
