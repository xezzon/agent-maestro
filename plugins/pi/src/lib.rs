//! 内置 pi 插件：把 Maestro 录入的 Provider 投影为 `~/.pi/agent/models.json`。
//!
//! 只依赖 `maestro-plugin-sdk`（`maestro:plugin` 合同的类型化绑定，见 issue #41），
//! 实现 SDK re-export 的 `Guest` trait；本插件兼作 SDK 的常驻契约验证与插件作者的
//! 参考实现。宿主把 manifest 声明的 config_dir（pi 即 `$HOME/.pi`）预开放为 "/"，
//! 插件以相对路径写入文件，整文件重写，保证已删除的 Provider 不残留。
//!
//! 端点选择在插件侧完成（ADR 0016）：宿主把全部端点与界面选择整包交给插件，
//! 由插件按自身兜底规则挑出投影使用的那个。

use std::fs;
use std::path::Path;

use maestro_plugin_sdk::{
    Endpoint, Guest, Level, Model, ModelCapability, Protocol, Provider, export, log,
};

/// pi 的 models.json 中 provider 条目的字段名。
const KEY_API: &str = "api";
const KEY_API_KEY: &str = "apiKey";
const KEY_BASE_URL: &str = "baseUrl";
const KEY_MODELS: &str = "models";
const KEY_MODEL_CONTEXT_WINDOW: &str = "contextWindow";
const KEY_MODEL_ID: &str = "id";
const KEY_MODEL_INPUT: &str = "input";
const KEY_MODEL_MAX_TOKENS: &str = "maxTokens";
const KEY_MODEL_NAME: &str = "name";
const KEY_MODEL_REASONING: &str = "reasoning";
const KEY_PROVIDERS: &str = "providers";

struct PiPlugin;

impl Guest for PiPlugin {
    fn write_providers(providers: Vec<Provider>) -> Result<Vec<String>, String> {
        write_models_json(&providers)?;
        Ok(vec!["agent/models.json".to_owned()])
    }
}

fn write_models_json(providers: &[Provider]) -> Result<(), String> {
    fs::create_dir_all(Path::new("agent"))
        .map_err(|e| format!("创建 agent 目录失败\n原因：{e}"))?;

    // serde_json 默认以 BTreeMap 承载对象，key 输出即按字典序排序，保证产物确定性。
    let mut root = serde_json::Map::new();
    for provider in providers {
        // 宿主只把非空端点的 Provider 送进来；其余属防御分支，不静默丢弃。
        let Some(endpoint) = effective_endpoint(provider) else {
            log(
                Level::Warning,
                &format!(
                    "Provider \"{}\" has no usable endpoint; skipping",
                    provider.slug
                ),
            );
            continue;
        };
        let mut entry = serde_json::Map::new();
        entry.insert(KEY_API.to_owned(), protocol_api(&endpoint.protocol).into());
        entry.insert(KEY_BASE_URL.to_owned(), endpoint.base_url.as_str().into());
        if let Some(api_key) = &provider.api_key {
            entry.insert(KEY_API_KEY.to_owned(), escape_api_key(api_key).into());
        }
        entry.insert(KEY_MODELS.to_owned(), models_json(&provider.models));
        root.insert(provider.slug.clone(), entry.into());
    }

    let doc = serde_json::json!({ KEY_PROVIDERS: root });
    let mut json = serde_json::to_string_pretty(&doc)
        .map_err(|e| format!("序列化 models.json 失败\n原因：{e}"))?;
    json.push('\n');

    // 原子写入：先写临时文件成功后再 rename，I/O 错误不会留下半截的 models.json。
    let tmp = Path::new("agent/models.json.tmp");
    fs::write(tmp, &json).map_err(|e| format!("写入 agent/models.json 临时文件失败\n原因：{e}"))?;
    fs::rename(tmp, Path::new("agent/models.json"))
        .map_err(|e| format!("替换 agent/models.json 失败\n原因：{e}"))?;

    Ok(())
}

/// 选择投影端点（ADR 0016）：
/// 1. 界面所选协议确有端点 → 用它；
/// 2. 否则只剩一个端点 → 用它（覆盖旧配置无选择、选择失效两类情形）；
/// 3. 否则多端点且无有效选择 → 取固定顺序里的首选 openai-completions。
fn effective_endpoint(provider: &Provider) -> Option<&Endpoint> {
    if let Some(selected) = &provider.selected_protocol
        && let Some(endpoint) = provider
            .endpoints
            .iter()
            .find(|endpoint| &endpoint.protocol == selected)
    {
        return Some(endpoint);
    }
    if provider.endpoints.len() == 1 {
        return provider.endpoints.first();
    }
    provider
        .endpoints
        .iter()
        .find(|endpoint| matches!(endpoint.protocol, Protocol::OpenaiCompletions))
        .or_else(|| provider.endpoints.first())
}

/// 枚举透传为 pi 期望的线协议名字符串。
fn protocol_api(protocol: &Protocol) -> &'static str {
    match protocol {
        Protocol::OpenaiCompletions => "openai-completions",
        Protocol::AnthropicMessages => "anthropic-messages",
    }
}

/// pi 的值解析规则：以 `$` 或 `!` 开头的字面值需要转义为 `$$` / `$!`。
/// 无凭证时宿主传 None，本地网关模型在 pi 中可见，用户可 /login 兜底。
fn escape_api_key(api_key: &str) -> String {
    if api_key.starts_with('$') || api_key.starts_with('!') {
        format!("${api_key}")
    } else {
        api_key.to_owned()
    }
}

/// 模型列表：display-name 有值且非空才写 name 字段，否则省略。
///
/// token 上限与能力（issue #111）同样有值才写：未设置一律省略，绝不写
/// null / false / 空数组——pi 的 contextWindow / maxTokens 是
/// `exclusiveMinimum: 0` 的数字，0 会被 pi 判为非法。非正值只可能来自手工
/// 改过的 config.json，跳过时刻意不记日志（与本文件「无可用端点即跳过」那条
/// 会记警告的防御分支不同）。pi 无对应字段的 limit.max_input 与
/// capabilities.tool_use 不投影；maestro 的 header 只落在 provider 级，
/// pi 的 model 级 headers 不使用。
fn models_json(models: &[Model]) -> serde_json::Value {
    models
        .iter()
        .map(|model| -> serde_json::Value {
            let mut entry = serde_json::Map::new();
            entry.insert(KEY_MODEL_ID.to_owned(), model.id.as_str().into());
            if let Some(name) = &model.display_name
                && !name.is_empty()
            {
                entry.insert(KEY_MODEL_NAME.to_owned(), name.as_str().into());
            }
            if let Some(context_window) = positive(model.limit.context_window) {
                entry.insert(KEY_MODEL_CONTEXT_WINDOW.to_owned(), context_window.into());
            }
            if let Some(max_output) = positive(model.limit.max_output) {
                entry.insert(KEY_MODEL_MAX_TOKENS.to_owned(), max_output.into());
            }
            // 能收图即两种模态都写：pi 的 input 是完整列表，没有「只收图不收字」的模型。
            if model.capabilities.contains(&ModelCapability::ImageIn) {
                entry.insert(
                    KEY_MODEL_INPUT.to_owned(),
                    serde_json::json!(["text", "image"]),
                );
            }
            if model.capabilities.contains(&ModelCapability::Thinking) {
                entry.insert(KEY_MODEL_REASONING.to_owned(), true.into());
            }
            entry.into()
        })
        .collect()
}

/// 声明式上限只在为正时才投影：0 是唯一可能的非正取值（`u32` 承载，
/// 负值在记录层即被类型拒绝），跳过且不记日志。
fn positive(limit: Option<u32>) -> Option<u32> {
    limit.filter(|limit| *limit > 0)
}

export!(PiPlugin);
