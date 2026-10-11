# Claude Code 适配器（`plugins/claude-code`）

> 状态：已实现（决策见 [ADR 0020](../../docs/adr/0020-provider-selection-in-plugin-form.md)、[ADR 0021](../../docs/adr/0021-tool-decides-protocol.md)，背景见 issue #56）。本文即本插件的投影合同，实现见 [`src/lib.rs`](src/lib.rs)。

把 Maestro 的 Provider 投影进 Claude Code 的**用户级** `~/.claude/settings.json`，只维护其 `env` 块里的四个键：

| 键 | 取值 |
| --- | --- |
| `ANTHROPIC_BASE_URL` | 投影所用 Provider 的 `anthropic-messages` 端点，原样写入 |
| `ANTHROPIC_AUTH_TOKEN` | Provider 的 `api_key`（**裸值**，Claude Code 自行加 `Bearer ` 前缀）；为空则不写该键 |
| `ANTHROPIC_MODEL` | 插件表单里的 `model_id`；未填则不写该键 |
| `ANTHROPIC_CUSTOM_HEADERS` | Provider 的 `custom_header`：`name: value` 按 name 排序后以 `\n` 连接；为空则不写该键 |

## 端点与协议

Claude Code 只对接 Anthropic Messages，且把 `ANTHROPIC_BASE_URL` 当作基址、自行追加 `/v1/messages`。因此：

- 候选 Provider = **已启用**且配了非空 `anthropic-messages` 端点的 Provider；投影一律用该 anthropic 端点，**不套用 ADR 0016 的兜底链、不看 `selected_protocol`**（见 ADR 0021）。
- 端点值**原样写入，不做任何规整**：不去末尾 `/`、不剥 `/v1`、不改路径前缀。请在 Provider 里填 Claude Code 的 host 根（如 `https://llm-gateway.example.com`）；填成 `.../v1` 会被拼成 `.../v1/v1/messages`，运行期打不通。

## 用哪个 Provider

Claude Code 同时只能对接一条接入，而宿主把全部 Provider 交给插件，所以选择由本插件的表单承载（见 ADR 0020）。表单把 `provider_slug` 列为**必填**（不保存空值），下表后三行的「表单留空」因此只在手工编辑配置文件时出现：

| 情形 | 行为 |
| --- | --- |
| 表单填了 `provider_slug`，且该 Provider 在候选内 | 投影它 |
| 表单填了，但该 Provider 没有 anthropic 端点 | 本次投影**失败**：该 Provider 未配置 `anthropic-messages` 端点，Claude Code 只能对接 Anthropic Messages |
| 表单填了，但该 Provider 不存在／已禁用／零端点 | 本次投影**失败**，原因列出候选 slug |
| 表单留空，候选恰好一条 | 投影它 |
| 表单留空，候选零条 | 清空本插件维护的四个键，视为成功 |
| 表单留空，候选多于一条 | 本次投影**失败**，原因列出候选 slug |

只配 `openai-completions` 的 Provider 与本插件无关：既不在候选内，也不产生任何写入。

`model_id` 同理由表单承载：所选 Provider 的模型列表非空时，值必须在该列表内，否则本次投影失败并列出可用 id；列表为空时不校验，照写。

## `settings.json` 是用户文件

`~/.claude/settings.json` 同时由 Claude Code 自己写入（`/config` 里归 user settings 的项，如主题），不是本插件独占的文件：

- 读入现有文件（不存在则视为 `{}`），**只增删上表那四个键**，其余键名与值（含嵌套）逐字保留、键的相对顺序不变（`serde_json` 的 `preserve_order`）。
- 承诺是**值 + 键序**，不是字节级：缩进与空行会被重排。settings 文件是严格 JSON（注释与尾逗号都是语法错误，官方会报 `Settings Error`），因此没有需要保住的人写格式。
- 移除四个键后 `env` 变成空对象时，连 `env` 一并删掉。
- 文件不存在、且**有目标状态**（有键要写或有键要删）时创建它；没有目标状态时既不创建也不写。
- 不做幂等比对：有目标状态就重写（tmp + rename 原子写），即使内容与上次相同。
- 下列任一前提不成立即**本次投影失败、完全不碰文件**：文件不是合法 JSON、根不是对象、`env` 存在但不是对象。失败时上次投影写入的键会留在原处（作为「上次已知可用值」），不会被顺手清掉。

## 其他约定

- 只写 `settings.json`。`config_dir` 是 `$HOME/.claude` 整个目录，但目录内其他文件一概不碰；同级的 `~/.claude.json` 更是插件沙箱之外的路径。
- 只写**用户级** settings。项目级 `.claude/settings.json`、`.claude/settings.local.json` 与企业 managed settings 的优先级更高，会压制本插件的投影（managed settings 一旦设了 `forceLoginMethod` 或 `forceLoginOrgUUID`，它就无法与 `ANTHROPIC_AUTH_TOKEN` 共存，本插件的投影在该机器上不会生效）。
- 不写 `ANTHROPIC_API_KEY`，也不碰用户已有的同名键；不使用 `apiKeyHelper`。用户已有的 `ANTHROPIC_API_KEY` 不会被清除，它会在交互式会话里触发一次「Use custom API key」询问。
- 官方把路由／租户类自定义 header 计入「需要批准的 header」，交互式会话可能为此再多问一次。
- **禁用本插件不会清除已投影的键**（禁用后插件不参与投影）。要清空受管键，先把可用 Provider 减到零再投影一次，或手工删除。
- 组件**不得**把凭证（API Key、header 值）写进 `logger` 的消息——见 [crates/maestro-plugin-sdk/README.md](../../crates/maestro-plugin-sdk/README.md)。
- 错误以人类可读字符串返回，显示在投影报告中；返回相对 `config_dir` 的已写入路径列表，无写入时为 `[]`。
