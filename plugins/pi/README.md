# pi 适配器（`plugins/pi`）

把 Maestro 的 Provider 投影进 pi 的 `~/.pi/agent/models.json`：**整文件重写**（tmp + rename），每次投影以 Maestro 为准，工具侧的手工改动会被覆盖。

投影用哪个端点按 ADR 0016 裁定：`selected-protocol` 有效则用它，否则唯一端点，多端点且无有效选择时优先 `openai-completions`。零端点的 Provider 由宿主在投影前跳过，不会到达插件。构建与沙箱规则见 [`crates/maestro-plugin-sdk/README.md`](../../crates/maestro-plugin-sdk/README.md)。

## 字段映射

| Maestro | pi `models.json` | 说明 |
| --- | --- | --- |
| 投影所用端点 | provider `baseUrl` + `api` | 端点 URL 写 `baseUrl`，其协议名写 `api`（`openai-completions` / `anthropic-messages`）。 |
| Provider `api_key` | provider `apiKey` | 未设置（空串）时省略该键。 |
| Provider `custom_header` | provider `headers` | 键原样写入，值按下一节转义后写入。只用 provider 级；pi 的 model 级 `headers` 不使用。 |
| Provider 的 `modelOverrides` | —— | Maestro 不写；模型元数据全部写在 `models[]` 里。 |
| Model `id` | `models[].id` | |
| Model `display_name` | `models[].name` | 空或缺失时省略该键。 |
| Model `limit.context_window` | `models[].contextWindow` | |
| Model `limit.max_output` | `models[].maxTokens` | |
| Model `capabilities.image_in` | `models[].input` = `["text","image"]` | 未设置时不写 `input`（不写「只有 text」的显式数组）。 |
| Model `capabilities.thinking` | `models[].reasoning` = `true` | 未设置时不写该键。 |
| Model `limit.max_input`、`capabilities.tool_use` | —— | pi 无对应字段，**不投影**。 |
| 保留字段 `cost.*` / `modalities.*` / `options` | —— | 本期不实现，不投影（pi 的对应物是 `cost`、`input`、`samplingParams`）。 |

**未设置一律省略**，绝不写 `null` / `false` / 空数组：pi 的 schema 里这些字段都是 optional，且 `contextWindow` / `maxTokens` 是 `exclusiveMinimum: 0` 的数字。若某模型的 `limit` 出现非正值（只可能来自手工改过的 `config.json`），**静默跳过**该值——不写进 pi，也不记日志（与本插件「无可用端点即跳过」那条会记警告的防御分支刻意不同）。

`context_window` 与 `max_output` 都未设置时不写相应键，pi 会据此降级（`contextWindow <= 0` 时关闭上下文压缩、不做裁剪）——Maestro 不编造默认值。这类「缺值」不视为错误，插件不发警告。

## 值的转义（两层）

pi 对 `apiKey` 与 **header 值**使用同一套解析：`$NAME` / `${NAME}` 取环境变量，前导 `!command` 执行命令，`$$` 表示字面 `$`，`$!` 表示字面前导 `!`；而且 `$` 在值的**任意位置**都特殊，不只看开头（`resolve-config-value.ts` 从 `index 0` 起反复 `indexOf("$")`）。所以值里中段的 `$` 也必须转义。

于是从用户书写到 pi 生效是两层：

1. **用户侧，宿主插值**（ADR 0014 / 0015）：值里的字面 `$` / `\` 写成 `\$` / `\\`；`${name}` / `$name` / `${name:default}` 引用全局变量。`custom_header` 的**值**参与插值，**键**不参与。
2. **投影侧，本插件**：把插值后的值转义成 pi 眼中的字面量——**每个 `$` 写成 `$$`**，若整个值以 `!` 开头再补一个 `$` 前缀。`api_key` 与 header 值共用同一个函数（原 `escape_api_key` 只处理前导 `$`，已按 #58 修全）。

因此用户写下 `\$FOO`，宿主插值得到 `$FOO`，插件写出 `$$FOO`，pi 最终得到字面 `$FOO`。

## 其他约定

- 返回相对 `config_dir` 的已写入路径列表，本插件返回 `agent/models.json`。
- 错误以人类可读字符串返回，显示在投影报告中。
- 组件**不得**把凭证（API Key、header 值）写进 `logger` 的消息——同上，见 SDK README。
