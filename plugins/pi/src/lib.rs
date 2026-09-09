//! 内置 pi 插件：把 Maestro 录入的 Provider 投影为 `~/.pi/agent/models.json`。
//!
//! 宿主把 manifest 声明的 config_dir（pi 即 `~/.pi`）预开放为 "/"，
//! 插件以相对路径写入文件，整文件重写，保证已删除的 Provider 不残留。

wit_bindgen::generate!({
    path: "../../wit",
    world: "plugin-world",
});

use std::fs;
use std::path::Path;

use exports::maestro::plugin::plugin::{Guest, Protocol, Provider};

/// pi 的 models.json 中 provider 条目的字段名。
const KEY_API: &str = "api";
const KEY_API_KEY: &str = "apiKey";
const KEY_BASE_URL: &str = "baseUrl";
const KEY_MODELS: &str = "models";
const KEY_MODEL_ID: &str = "id";
const KEY_MODEL_NAME: &str = "name";
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
        let mut entry = serde_json::Map::new();
        entry.insert(KEY_API.to_owned(), protocol_api(&provider.protocol).into());
        entry.insert(KEY_BASE_URL.to_owned(), provider.base_url.as_str().into());
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
fn models_json(models: &[exports::maestro::plugin::plugin::Model]) -> serde_json::Value {
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
            entry.into()
        })
        .collect()
}

export!(PiPlugin);
