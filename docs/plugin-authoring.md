# 插件作者指南

本指南面向第三方插件作者，覆盖 manifest 字段语义、https 发布流、本地调试回路，以及凭证投影（keychain 桥接）、`config_dir` 安全规则、沙箱边界、fuel 预算与原子写建议。

术语以 [`CONTEXT.md`](../CONTEXT.md) 为准；架构取舍见 [ADR 0004](adr/0004-wasm-component-plugins.md)（WASM 组件插件）、[ADR 0006](adr/0006-plugin-sources-https-and-local-path.md)（来源与发布）、[ADR 0007](adr/0007-plugin-sdk-distributed-via-git-tag.md)（SDK 分发）。

一个 Maestro 插件是一个 WASM 组件（Component Model / WASI 0.2），实现 `maestro:plugin` 合同导出的 `write-providers`：宿主把 Maestro 中录入的 Provider 交给插件，插件把它投影进 manifest 声明的 `config_dir`。插件只依赖 [`maestro-plugin-sdk`](../crates/maestro-plugin-sdk/README.md)（WIT 合同的类型化 Rust 绑定），接口不兼容在编译期暴露；依赖写法、最小骨架与版本对齐约定见该 crate 的 README。

第三方插件以两种来源安装：**https 来源**（指向 manifest.json 的 https URL，面向正式发布）与**file 来源**（指向本机 manifest.json 绝对路径，面向本地调试）。两种来源在装载、校验与沙箱上完全同构。

## manifest.json

manifest 是插件 metadata 的唯一来源：宿主直接读插件根目录的 `manifest.json`（内置插件的 manifest 内嵌于二进制），组件不导出 get-metadata。宿主容忍未知字段；必填字段缺失或为空即加载失败。

| 字段 | 必填 | 语义 |
| --- | --- | --- |
| `id` | 是 | 插件 id，规则与 Provider slug 一致：`[a-z][a-z0-9-_]*`。它是落位目录名、投影报告与 id 冲突检查的依据；界面展示也用它，作者应把 id 命名得足以看出适配哪类工具。 |
| `config_dir` | 是 | 插件被授权写入的配置目录，以**平台变量前缀 + 相对片段**声明（如 `$HOME/.pi`、`$XDG_CONFIG_HOME/zed`），见 [config_dir 声明语法](#config_dir-声明语法)。 |
| `entry` | 是 | 入口 wasm 的**回源地址**。约束按来源分列（见下）。 |
| `keychain_namespace` | 否 | 声明插件使用哪个凭证适配器（见 [凭证投影](#凭证投影keychain-桥接)）。声明后插件才允许调用 `keychain` import；未声明即调用报错。 |

`config_dir` 是一个**目录**：宿主按当前平台把它解析为绝对路径，并在投影时预开放为组件内的 `/`（插件唯一的写入面）。因此组件里写 `/agent/models.json` 就等于写 `config_dir/agent/models.json`。

`entry` 是回源地址：宿主「重新加载」时据此知道从哪里重新获取 wasm。落位时 manifest 原样保存、`entry` 不重写，本地 wasm 固定存为 `plugin.wasm`，装载只认这个固定名、不解析 `entry`。

### entry 按来源分列

同一份 manifest 在三种来源下对 `entry` 的约束不同，不做跨协议的统一解析：

| 来源 | `entry` 约束 |
| --- | --- |
| 内置于应用（`builtin:<id>`） | 不参与解析（wasm 内嵌于宿主二进制）。 |
| https URL | 必须是绝对 https URL（带主机）；明文 http 与其他 scheme 一律拒绝。 |
| 本机绝对路径（file） | 插件目录内的相对路径（相对 manifest.json 所在目录，可含子目录），或 https URL（回源到网络）。 |

通用拒绝项：**file 来源的 `entry` 若为相对路径，绝对路径、含 `..`、带非 https scheme（`http://`、`file://` 等）一律被拒绝**——任何 manifest 都不能让宿主读插件根之外的文件。file 来源的相对 entry 在读取时还会按规范化的真实路径确认没有借符号链接逃出插件目录。

### 两个例子

提交进仓库、供本地调试的 manifest（`entry` 指向 cargo 原生产物路径，产物不入库）：

```json
{
  "id": "pi",
  "config_dir": "$HOME/.pi",
  "entry": "target/wasm32-wasip2/release/maestro_plugin_pi.wasm"
}
```

发布到 Release 的 manifest（只存在于 Release 资产中，`entry` 指向同一个 Release 的 wasm 资产 URL）：

```json
{
  "id": "pi",
  "config_dir": "$HOME/.pi",
  "entry": "https://github.com/<owner>/<repo>/releases/download/v1.0.0/plugin.wasm"
}
```

## 凭证投影（keychain 桥接）

有些工具不把 API Key 写进自己的配置文件，只从**系统凭证库**读取（macOS Keychain / Windows Credential Manager / Linux Secret Service），条目按该工具自己的索引键存放。插件跑在沙箱里、够不到凭证库，因此这类凭证由宿主桥接：宿主把凭证库做成 WIT import，插件把 `(key, secret)` 交给宿主，由宿主按 manifest 声明的 `keychain_namespace` 落到目标工具自己的条目上。

```rust
use maestro_plugin_sdk::keychain;

// 有 key 就写、没有就删；key 是该工具侧的索引键，不是 Provider slug。
keychain::write("https://api.example.com/v1", "sk-...")?;
keychain::delete("https://api.example.com/v1")?;
```

约定与语义：

- **`key` 是目标工具的索引键**，含义由宿主的适配器决定（例如某工具按 API 端点 URL 精确匹配条目）。条目的其余字段（服务名、用户名、标签）由适配器补齐——插件不指定，也无从指定。
- **`keychain_namespace` 声明是权限闸门**：manifest 未声明该字段时调用 `write` / `delete` 一律报错；声明了宿主不认识的 namespace 同样在**调用时**报错（错误随返回值回到插件，可写进投影报告）。报错不影响插件的装载与配置文件投影。可用的 namespace 随宿主版本增加；宿主内置适配器当前为空，各工具的适配器随对应插件另立 issue 落地。
- `delete` 对不存在的 `key` 视为成功（幂等）；同 `key` 重复写入为覆盖（按适配器自身语义）。
- 凭证明文经 WIT 调用传入宿主，**不落磁盘明文**（宿主只把它交给系统凭证库；日志只记 namespace 与索引键）。错误消息里不要回显 `secret`——它会进投影报告与日志。

推荐的消费方式与配置文件投影一致地「逐条容错」：按宿主传入顺序遍历 Provider，有 `api_key` 的 `write`、没有的 `delete`；**单条失败即跳过该条**，继续处理其余条目与配置文件投影，并让失败原因随 `write-providers` 的返回值可见。索引键重复时后者覆盖前者。

已知限制：插件无状态，在 Maestro 里删除 Provider 后，其在目标工具钥匙串里的条目成为**孤儿**，不会自动清理——请在给用户的说明里提示到目标工具中手动重置该凭证。

## 发布流（https 来源）

约定：**作者打 tag → 构建产物挂 GitHub Release → 用户以规整的 Release manifest URL 安装**。宿主不感知 GitHub，只做纯 https 下载（跟随资产下载的重定向）。发布流由作者自备——本仓库不交付插件模板仓库或现成的 Release workflow。

一个版本的发布步骤：

1. 锁定工具链，构建 wasm：`cargo build --release --target wasm32-wasip2`。产物须是 WASM 组件，宿主会在安装与装载时做实例化校验。
2. 生成**发布版 manifest**：在仓库 manifest 的基础上把 `entry` 改写为本次 Release 的 wasm 资产 URL。仓库 manifest 的 `entry` 指向本地 target 路径、不是 https URL，不能直接当发布版 manifest 用。

   ```bash
   mkdir -p release
   jq --arg url "https://github.com/<owner>/<repo>/releases/download/$TAG/plugin.wasm" \
     '.entry = $url' manifest.json > release/manifest.json
   ```

3. 把构建产物以资产名 `plugin.wasm`、`release/manifest.json` 以资产名 `manifest.json` 挂到 GitHub Release。
4. 用户安装：在 Maestro「添加插件」里选「https 地址」，粘贴规整的 Release manifest URL：

   ```
   https://github.com/<owner>/<repo>/releases/download/<tag>/manifest.json
   ```

约定细节：

- **两个资产名固定为 `plugin.wasm` 与 `manifest.json`**，上面的 URL 模板与 `entry` 都据此成立。
- 发布版 manifest 只存在于 Release 资产、不回写仓库；发新版即打新 tag，Release 上的 manifest 不可变、可放心引用。
- 宿主只做纯 https 下载并跟随重定向（GitHub 资产下载会重定向到对象存储域名）。4xx/5xx、超过 64 MiB 的响应按失败处理。
- 安装失败时不写入配置条目、界面直接显示原因；修复后按同一来源重新添加即可，无需重新填 URL。

## 本地调试回路（file 来源）

开发回路是「**编译 → 重新加载**」：`entry` 直接指向 cargo 原生产物路径，构建完点一下「重新加载」即可，不必手动搬运产物。

1. 仓库 manifest 的 `entry` 指向 cargo 原生产物路径 `target/wasm32-wasip2/release/<crate>.wasm`（crate 名里的连字符在产物名中变为下划线，如 `maestro-plugin-pi` → `maestro_plugin_pi.wasm`）。不提交 wasm，仓库干净，产物也不会与源码漂移。
2. `cargo build --release --target wasm32-wasip2`
3. 在 Maestro「添加插件」里来源选「本地文件」，用文件选择器选中仓库根目录的 `manifest.json`。file 来源的来源身份是这个本机绝对路径。
4. 改码后重新编译，点「重新加载」——宿主按来源重新读 manifest、按 `entry` 取 wasm，落位目录替换为最新产物。开发回路不需要在 Maestro 里重填来源。

语义要点：

- file 来源与 https 来源同构：添加与重新加载都会按 `entry` 取 wasm（相对路径读来源目录内文件，https URL 联网获取），一并落位 `~/.maestro/plugins/<id>/`（`manifest.json` 原文 + `plugin.wasm`）。
- 「重新加载」成功才替换落位目录，失败时旧版本保持可用；上游 manifest 的 id 变更会报错并保持旧状态。
- **没有单独的「更新」操作**：重新加载就是按来源重新获取，每次只作用于一个来源。
- 应用启动只读落位目录、不联网、不读来源目录，因此 file 来源的插件离线也可装载。
- 「移除」只删配置条目与落位副本、幂等，**不动你的插件项目目录与构建产物**。
- file 来源的 `entry` 也可以是 https URL（例如指向 Release 产物），这样不本地构建也能调试发布版 manifest；此时添加与重新加载会联网获取。
- 落位是复制而非直读项目目录（与 https 来源同构的代价是每个插件占一份副本）；「重新加载」把最新产物复制进落位目录。

## 安全规则与沙箱

### config_dir 声明语法

`config_dir` = **平台变量前缀** + `/` + **相对片段**。变量按当前平台解析为该变量对应的基准目录，相对片段拼在其后——同一份声明因此在各平台各得其所（`$XDG_CONFIG_HOME/zed` 在 Linux 是 `~/.config/zed`、在 macOS 是 `~/Library/Application Support/zed`、在 Windows 是 `%APPDATA%\zed`）：

| 变量 | 含义 |
| --- | --- |
| `$HOME`（或 `~`，同义） | 主目录 |
| `$XDG_CONFIG_HOME` | 平台配置目录（Linux 遵循 XDG 环境变量，默认 `~/.config`；macOS `~/Library/Application Support`；Windows `%APPDATA%`） |
| `$LOCAL_APP_CONFIG` | 同上的「本地」变体（Windows `%LOCALAPPDATA%`，其余平台同上） |
| `$XDG_DATA_HOME` | 平台数据目录（Linux `~/.local/share`；macOS `~/Library/Application Support`；Windows `%APPDATA%`） |
| `$LOCAL_APP_DATA` | 同上的「本地」变体（Windows `%LOCALAPPDATA%`，其余平台同上） |

规则与拒绝项：

- 变量必须在最前、且是上表之一；变量之后必须有非空的相对片段（只写 `$HOME` 不算）。
- 相对片段不得是绝对路径、不得含 `..`、不得含裸 `.`（`$HOME/.`、`$HOME/./x` 这类与基准目录重合的写法被拒——把整盘 `$HOME` 交出去不在合同范围内）。
- 目录不存在时宿主先创建；创建后按规范化的真实路径确认仍**严格落**在该变量对应的基准目录内（与基准目录相等同样视为逃逸），借符号链接逃逸也在这道关里被拒。
- 违规即 manifest 校验失败：不落位、不写配置条目，界面直接显示原因。
- 不提供 cache 目录变量（当前没有工具需要写缓存）。

### 沙箱边界

- 插件在 wasmtime 宿主的 WASI 0.2 沙箱里运行，**唯一预开放目录是 manifest 声明的 `config_dir`（挂载为组件内的 `/`，读写权限）**。这是插件唯一的**文件系统**写入面；`config_dir` 之外的文件系统访问被沙箱拒绝。预开放目录之外，插件还能影响的状态只有经宿主桥接的目标工具系统凭证库（见 [凭证投影](#凭证投影keychain-桥接)），且以 manifest 声明的 `keychain_namespace` 为限。
- 宿主不代写文件：写入由插件在沙箱内经 WASI 直接落盘（因此 TOML、JSON、SQLite 等非纯文件配置天然可支持）；代价是宿主对写入内容零可见。
- 宿主与第三方插件使用完全一致的校验与沙箱（manifest 校验、`config_dir` 规则、实例化校验、id 冲突检查），不因来源放松。
- 信任模型是**安装即信任**，与自行安装 npm 包同级：沙箱隔离能力滥用，宿主不审查插件内容。
- 组件只导出 `write-providers`；宿主侧 import 只有 WASI 与 `keychain`（后者未声明 `keychain_namespace` 时调用即报错）。

### fuel 预算

宿主为每次插件调用设置 fuel 预算（`1 << 30` 量级的指令数）。死循环或超量计算会耗尽 fuel 被中断，该插件本次投影走失败路径并在报告中给出原因，不会卡死应用。正常投影任务的量级远小于该预算；但不要在插件里做重计算——等待 I/O 不消耗 fuel。预算不可配置。

### 原子写建议

落盘建议 **tmp + rename**：先写临时文件、成功后改名替换目标文件，这样 I/O 失败不会留下半截配置文件。参考实现 [`plugins/pi/src/lib.rs`](../plugins/pi/src/lib.rs) 就是整文件重写：先写 `agent/models.json.tmp`，再 rename 为 `agent/models.json`，保证已删除的 Provider 不残留。

## 参考实现

内置 pi 插件 [`plugins/pi`](../plugins/pi) 是参考实现，兼作 SDK 的常驻契约回归，建议对照它实现：

- **整文件重写**目标配置，而非增量修改，避免已删除条目残留。
- 写入用 **tmp + rename**。
- `write-providers` 返回**相对 `config_dir`** 的已写入文件路径列表。
- 错误以人类可读字符串返回，会显示在投影报告中。

投影合同的完整定义（输入输出类型、单协议 Provider、宿主跳过规则）见 WIT 合同 [`crates/maestro-plugin-sdk/wit/maestro-plugin.wit`](../crates/maestro-plugin-sdk/wit/maestro-plugin.wit)。
