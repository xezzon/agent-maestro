# 各 Agent 工具的文件级模型 / Provider 配置字段支持矩阵

调研日期：2026-10-07。范围：仅限**以文件承载模型配置**的部分字段。用于评估 Maestro 通用模型元数据应覆盖哪些字段。

更正（2026-10-10）：Pi 列经源码核对后修正——`models.json` 的 schema 实际支持 provider 级 `headers`、model 级 `contextWindow` / `maxTokens` / `cost`（`input`/`output`/`cacheRead`/`cacheWrite`）/ `input`（模态数组），且在 `providers.<id>.models[]` 与 `modelOverrides` 两处都可写；原表把上下文窗口、价格标为「目录」、把 custom headers 标为「?」是错的。逐条见下表与「逐工具说明」。

图例：

- `✓` 有专门的配置字段（可写进该工具的配置文件）。
- `◐` 间接 / 部分：经由透传（如 `options` / `extra_params`）、能力开关，或只存在于该工具的模型**目录元数据**（models.dev / catalog / 托管价目表）而非其配置文件。
- `·` 不适用：该工具不使用此机制（例如 Aider 不用 tool calling，Zed 不把 api key 写进 settings.json）。
- `?` 无法核实（官方文档未提及；**不代表**一定不支持）。

## 支持矩阵

| 字段 | Pi | OpenCode | Zed | Continue | Cline | Roo/Kilo | Aider |
|---|---|---|---|---|---|---|---|
| 上下文窗口 / max input tokens | ✓ `contextWindow`（**无** max input 字段） | ✓ `limit.context` | ✓ `max_tokens` | ✓ `contextLength` | ✓ Context Window | ✓ `limit.context` | ✓ `max_input_tokens` |
| max output tokens | ✓ `maxTokens` | ✓ `limit.output` | ✓ `max_output_tokens` | ✓ `maxTokens` | ✓ Max Output Tokens | ✓ `limit.output` | ✓ `max_output_tokens` |
| 输入模态 text/image | ✓ `input` | ? | ✓ `supports_images` | ✓ `image_input` | ✓ Image Support | ✓ `modalities.input` | ? |
| 输入模态 audio/video/pdf | ? | ? | ? | ? | ? | ✓ `modalities.input` | ? |
| 输出模态 | ? | ? | ? | ? | ? | ✓ `modalities.output` | ? |
| reasoning effort | ✓ thinking level | ✓ `reasoningEffort` | ✓ `reasoning_effort` | ? | ? | ✓ `reasoning` + variants | ✓ `reasoning_effort` |
| reasoning budget tokens | ? | ✓ `thinking.budgetTokens` | ✓ `mode.budget_tokens` | ✓ `reasoningBudgetTokens` | ? | ✓ `options.thinking` | ✓ `thinking_tokens` |
| tool/function calling 能力标志 | ? | ? | ✓ `capabilities.tools` | ✓ `tool_use` | ◐ Computer Use | ✓ `tool_call` | · |
| structured output / json schema | ? | ? | ? | ? | ? | ? | ? |
| streaming 开关 | ? | ? | ? | ? | ? | ? | ✓ `streaming` |
| parallel tool calls | ? | ? | ✓ `capabilities.parallel_tool_calls` | ? | ? | ? | · |
| temperature | ✓ `samplingParams` | ◐ `options` | ✓ `default_temperature` | ✓ `temperature` | ◐ profile | ✓ `options` / profile | ◐ `extra_params` |
| top_p / top_k | ✓ `samplingParams` | ◐ `options` | ? | ✓ `topP` / `topK` | ? | ◐ `options` | ◐ `extra_params` |
| penalties / seed / stop | ◐ `samplingParams` | ◐ `options` | ? | ◐ `stop` | ? | ◐ `options` | ◐ `extra_params` |
| pricing / cost (input/output) | ✓ `cost` | ? | ? | ? | ✓ Input/Output Price | ✓ `cost` | ✓ `*_cost_per_token` |
| cache 读写价格 | ✓ `cost.cacheRead` / `cost.cacheWrite` | ? | ◐ 托管表 | ? | ? | ✓ `cost.cache_read/write` | ? |
| knowledge cutoff | ? | ? | ? | ? | ? | ? | ? |
| release date | ? | ? | ? | ? | ? | ? | ? |
| aliases | ? | ? | ? | ? | ? | ? | ? |
| deprecation | ? | ? | ? | ? | ? | ? | ? |
| prompt caching 配置 | ✓ `promptCache` | ◐ `setCacheKey` | ✓ `prompt_caching` | ? | ? | ◐ cache controls | ✓ `cache_control` |
| custom HTTP headers | ✓ `headers`（provider 级与 model 级） | ✓ `options.headers` | ✓ `custom_headers` | ✓ `requestOptions.headers` | ? | ✓ `headers` | ✓ `extra_params.extra_headers` |
| api key | ✓ `apiKey` | ✓ `options.apiKey` | · keychain/env | ✓ `apiKey` | ◐ Secret Storage | ✓ `options.apiKey` / `env` | · env |
| base url | ✓ `baseUrl` | ✓ `options.baseURL` | ✓ `api_url` | ✓ `apiBase` | ✓ Base URL | ✓ `options.baseURL` | · |
| compatibility flags / protocol | ◐ | ✓ `npm` / protocol | ✓ `capabilities` | ◐ `provider` | ◐ provider API | ✓ `npm` / `provider` | ◐ `edit_format` |
| display name / id 映射 | ◐ `id` | ✓ `name` / `id` | ✓ `name` / `display_name` | ✓ `name` / `model` | ◐ Model ID | ✓ `name` / `id` | ✓ `name` |

## 逐工具说明

### Pi — `models.json`

来源：<https://pi.dev/docs/latest/models>；字段以源码 schema 为准——`packages/coding-agent/src/core/model-config.ts`（provider / model）、`packages/ai/src/providers/model-schema.ts`（嵌套 schema）、`packages/ai/src/providers/compat-schema.ts`（`compat`）。

- Provider 级：`name`、`baseUrl`、`apiKey`、`api`（协议；另有 `oauth`、`authHeader`）、`headers`、`compat`、`models`、`modelOverrides`。
- Model 级（`models[]` 与 `modelOverrides` 同名同义）：`id`、`name`、`reasoning`、`thinkingLevelMap`、`input`（`text` / `image`）、`inputLimits`（`maxRequestBytes`、`images.resize.maxWidth/maxHeight/maxBytes/jpegQuality`、`images.maxPerMessage/maxPerRequest`）、`cost {input, output, cacheRead, cacheWrite, tiers?}`（USD per million tokens）、`contextWindow`、`maxTokens`、`promptCache {short, long}`、`samplingParams`（free-form）、`samplingParamsByThinkingLevel`、`headers`、`compat`。差异：`modelOverrides` 没有 `id`/`api`/`baseUrl`，且 `cost` 各费率可选。
- 模型 **catalog** 提供内建模型的默认元数据；`models.json` 的 `models[]` 可**定义**模型、`modelOverrides` 可**覆盖**内建模型的元数据——`contextWindow` / `maxTokens` / `cost` 等因此在用户配置文件里可写，不再只是「目录」。
- 运行时用法：`contextWindow` 决定上下文压缩阈值与溢出判定，并把输出上限 clamp 到「上下文窗口 − 预估输入 − 4096」；`maxTokens` 直接成为请求的 `max_tokens` / `max_completion_tokens`（`packages/ai/src/api/simple-options.ts` 等）。

### OpenCode — `opencode.json(c)`

来源：<https://opencode.ai/docs/config/>、<https://opencode.ai/docs/providers/>

- Provider (`provider.<id>`)：`options`（`apiKey`、`baseURL`、`headers`、`timeout`、`headerTimeout`、`chunkTimeout`、`setCacheKey`；Bedrock 另有 `region`/`profile`/`endpoint`）、`npm`、`name`、`models`、`blacklist` / `whitelist`。
- Model (`provider.<id>.models.<id>`)：`name`、`id`、`limit {context, output}`、`options`（任意透传，如 `reasoningEffort`/`textVerbosity`/`reasoningSummary`/`include`/`thinking.budgetTokens`）、`variants`、`reasoning`、`interleaved`。
- 模型能力 / 模态 / 价格来自 **models.dev**，不落在配置文件里。

### Zed — `settings.json`

来源：<https://zed.dev/docs/ai/use-api-access>

- `language_models.<provider>.available_models[]`：`name`、`display_name`、`max_tokens`（上下文）、`max_output_tokens` / `max_completion_tokens`、`reasoning_effort`、`mode {type: thinking, budget_tokens}`、`tool_override`、`default_temperature`、`extra_beta_headers`、`supports_tools`、`supports_images`、`capabilities {tools, images, parallel_tool_calls, prompt_cache_key, prompt_caching, chat_completions, interleaved_reasoning, max_tokens_parameter}`。OpenCode 自定义模型另有 `protocol`、`reasoning_effort_levels`、`custom_model_api_url`。
- Provider 级：`api_url`、`custom_headers`。
- **API key 不写进 `settings.json`**：走系统 keychain 或 `<PROVIDER>_API_KEY` 环境变量。

### Continue — `config.yaml`（`config.json` 已废弃）

来源：<https://docs.continue.dev/reference>

- `models[]`：`name`、`provider`、`model`、`apiBase`、`apiKey`、`roles`、`capabilities`（`tool_use` / `image_input`）、`maxStopWords`、`promptTemplates`、`chatOptions`、`defaultCompletionOptions`（`contextLength` / `maxTokens` / `temperature` / `topP` / `topK` / `stop` / `reasoning` / `reasoningBudgetTokens` / `keepAlive`）、`requestOptions`（`headers` / `timeout` / `proxy` / `extraBodyProperties` / `verifySsl` / `caBundlePath` …）、`autocompleteOptions`、`embedOptions`、`useLegacyCompletionsEndpoint`。
- 密钥用 `${{ secrets.X }}`。无模态、价格、缓存等字段。

### Cline（VS Code 扩展）

来源：<https://docs.cline.bot/provider-config/openai-compatible>

- 模型配置经 **UI**（非可移植配置文件；API Key 存于 VS Code Secret Storage）：Max Output Tokens、Context Window、Image Support、Computer Use、Input Price、Output Price。
- Provider 级：Base URL、API Key、Model ID、Use Azure Identity Authentication。
- 其余能力字段（模态、缓存、headers 等）未文档化。

### Roo Code / Kilo Code

Roo 来源：<https://docs.roocode.com/providers/openai-compatible>、<https://docs.roocode.com/features/api-configuration-profiles>
Kilo 来源：<https://kilocode.ai/docs/code-with-ai/agents/custom-models>、<https://kilocode.ai/docs/ai-providers/openai-compatible>

- **Roo** 的 “Model Configuration”（**UI**）：Max Output Tokens、Context Window、Image Support、Computer Use、Input/Output Price；另有 temperature、thinking budget、provider-specific settings、rate limit、`apply_diff`。
- **Kilo**（基于 CLI 的 `kilo.jsonc`，`provider.<id>.models.<id>`）字段最全：`name`、`id`、`tool_call`、`reasoning`、`temperature`（能力开关，值是 boolean）、`attachment`、`modalities {input, output}`（`text`/`image`/`audio`/`video`/`pdf`）、`limit {context, output, input}`、`cost {input, output, cache_read, cache_write}`、`options`（任意透传，含 `thinking.budgetTokens`）、`headers`、`provider {npm, api}`、`variants`；Provider 级 `options {apiKey, baseURL, timeout, chunkTimeout}`、`env`、`whitelist` / `blacklist`。
- Kilo 是 Roo 的重写 / 延续，文件级字段以 Kilo 文档为准；仍无 structured output、parallel tool calls、knowledge cutoff 等字段。

### Aider

来源：<https://aider.chat/docs/config/adv-model-settings.html>

- 两个文件，经 `--model-metadata-file` / `--model-settings-file`（或默认位置）加载：
  - `.aider.model.metadata.json`：`max_tokens`、`max_input_tokens`、`max_output_tokens`、`input_cost_per_token`、`output_cost_per_token`、`litellm_provider`、`mode`。
  - `.aider.model.settings.yml`：`edit_format`、`weak_model_name`、`use_repo_map`、`send_undo_reply`、`lazy`、`overeager`、`reminder`、`examples_as_sys_msg`、`extra_params`、`cache_control`、`caches_by_default`、`use_system_prompt`、`use_temperature`、`streaming`、`editor_model_name`、`editor_edit_format`、`reasoning_tag`、`remove_reasoning`、`system_prompt_prefix`、`accepts_settings`。
- `temperature` / `top_p` 等经 `extra_params` 透传；模型元数据主体来自 **litellm**。无 tool calling，无 base url / api key（走环境变量）。

## 对 Maestro 建模的要点

- **跨工具共识最强的字段**：上下文窗口、max output tokens、temperature、base url、api key、custom headers —— 各工具的配置文件几乎都直接承载。
- **能力 / 模态 / 价格在 Pi、OpenCode、Zed 上多为“目录”**（catalog / models.dev / 托管价目表），配置文件本身不承载：Maestro 若要建模这些，需区分「目录来源」与「用户覆盖」两种语义。
- **Zed 的粒度最贴近通用模型元数据**：把用户可覆盖项拆进 model 级 `available_models`（含 `capabilities` 子对象）。
- **Kilo 的数据形状最接近通用模型元数据**：`modalities` / `limit` / `cost` 三者可直接对应输入输出模态、token 上限、价格。
- knowledge cutoff、release date、aliases、deprecation 在**所有** 7 个工具中均未文档化（一律 `?`）；structured output / json schema 同样全部为 `?`。
