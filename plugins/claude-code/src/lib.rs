//! 内置 Claude Code 插件：把 Maestro 录入的 Provider 投影为
//! `~/.claude/settings.json` 的 `env` 块。
//!
//! 投影合同（写哪些键、怎么选 Provider、失败语义、合并写入）见本目录的
//! README.md，决策见 ADR 0020、ADR 0021。宿主把 manifest 声明的 config_dir
//! （Claude Code 即 `$HOME/.claude`）预开放为 "/"，插件只写其中的
//! settings.json；该文件由 Claude Code 自身一并写入，故只增删本插件维护的键。

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use serde::Deserialize;

use maestro_plugin_sdk::{CustomHeader, Endpoint, Guest, Protocol, Provider, export, get_config};

/// 投影目标：`config_dir` 内的用户级 settings。
const SETTINGS_FILE: &str = "settings.json";
/// 原子写的同目录临时文件。
const SETTINGS_TMP: &str = "settings.json.tmp";

/// 本插件维护的键；顺序即首次写入时的落盘顺序。
const KEY_BASE_URL: &str = "ANTHROPIC_BASE_URL";
const KEY_AUTH_TOKEN: &str = "ANTHROPIC_AUTH_TOKEN";
const KEY_MODEL: &str = "ANTHROPIC_MODEL";
const KEY_CUSTOM_HEADERS: &str = "ANTHROPIC_CUSTOM_HEADERS";
const MANAGED_KEYS: [&str; 4] = [KEY_BASE_URL, KEY_AUTH_TOKEN, KEY_MODEL, KEY_CUSTOM_HEADERS];

/// 插件表单（ADR 0017 / 0018）：两个字段都可选，缺省即按确定性规则选 Provider。
#[derive(Debug, Default, Deserialize)]
struct Form {
    #[serde(default)]
    provider_slug: Option<String>,
    #[serde(default)]
    model_id: Option<String>,
}

struct ClaudeCodePlugin;

impl Guest for ClaudeCodePlugin {
    fn write_providers(providers: Vec<Provider>) -> Result<Vec<String>, String> {
        let form = read_form()?;
        let desired = select_env(&providers, &form)?;
        write_settings(&desired)
    }
}

/// 读取插件表单（ADR 0018）：宿主只透传 `form` 段的原始 JSON 文本，形状按
/// manifest 的 settings_schema 声明。缺省即两个字段都留空；`null` 视同缺省。
fn read_form() -> Result<Form, String> {
    let Some(raw) = get_config() else {
        return Ok(Form::default());
    };
    Ok(serde_json::from_str::<Option<Form>>(&raw)
        .map_err(|e| format!("插件配置的 form 段不是合法 JSON\n原因：{e}"))?
        .unwrap_or_default())
}

/// 选出投影目标并算出受管键的目标值（见 README「用哪个 Provider」）。
/// 返回空表示没有键要写（零候选）。
fn select_env(providers: &[Provider], form: &Form) -> Result<Vec<(&'static str, String)>, String> {
    let provider_slug = form
        .provider_slug
        .as_deref()
        .filter(|slug| !slug.is_empty());
    let model_id = form.model_id.as_deref().filter(|id| !id.is_empty());

    // 候选 = 配了非空 anthropic-messages 端点者，连同端点一并留用，避免下面重复查找。
    // 宿主只交付已启用且非零端点的 Provider，故这里不再看 enabled；也不套用 ADR 0016
    // 的兜底链、不看 selected_protocol（见 ADR 0021）。
    let candidates: Vec<(&Provider, &Endpoint)> = providers
        .iter()
        .filter_map(|provider| anthropic_endpoint(provider).map(|endpoint| (provider, endpoint)))
        .collect();

    let selected = match provider_slug {
        Some(slug) => {
            let provider = providers
                .iter()
                .find(|provider| provider.slug == slug)
                .ok_or_else(|| {
                    format!(
                        "插件表单指定的 Provider「{slug}」不可用：{}",
                        candidate_hint(&candidates)
                    )
                })?;
            let endpoint = anthropic_endpoint(provider).ok_or_else(|| {
                format!(
                    "Provider「{slug}」未配置 anthropic-messages 端点，Claude Code 只能对接 Anthropic Messages"
                )
            })?;
            Some((provider, endpoint))
        }
        None => match candidates.as_slice() {
            [] => None,
            [only] => Some(*only),
            many => {
                return Err(format!(
                    "检测到 {} 个 anthropic-messages Provider（{}），Claude Code 同时只能对接一条接入，请在插件配置里指定 provider_slug",
                    many.len(),
                    slug_list(many)
                ));
            }
        },
    };

    // 零候选：清空受管键（README：视为成功）。
    let Some((provider, endpoint)) = selected else {
        return Ok(Vec::new());
    };

    if let Some(model_id) = model_id {
        let models = &provider.models;
        if !models.is_empty() && !models.iter().any(|model| model.id == model_id) {
            return Err(format!(
                "模型「{model_id}」不在 Provider「{}」的模型列表内；可用模型：{}",
                provider.slug,
                models
                    .iter()
                    .map(|model| model.id.as_str())
                    .collect::<Vec<_>>()
                    .join("、")
            ));
        }
    }

    Ok(desired_env(provider, endpoint, model_id))
}

/// 目标受管键（按 MANAGED_KEYS 顺序）：端点原样写入；凭证为空、模型未填、
/// header 为空时各自不写该键。
fn desired_env(
    provider: &Provider,
    endpoint: &Endpoint,
    model_id: Option<&str>,
) -> Vec<(&'static str, String)> {
    let mut entries = vec![(KEY_BASE_URL, endpoint.base_url.clone())];
    if let Some(api_key) = provider.api_key.as_deref().filter(|key| !key.is_empty()) {
        entries.push((KEY_AUTH_TOKEN, api_key.to_owned()));
    }
    if let Some(model_id) = model_id {
        entries.push((KEY_MODEL, model_id.to_owned()));
    }
    let headers = format_headers(&provider.custom_header);
    if !headers.is_empty() {
        entries.push((KEY_CUSTOM_HEADERS, headers));
    }
    entries
}

/// 该 Provider 的 anthropic 端点；未配置即 `None`。每协议至多一个端点（WIT 合同）。
fn anthropic_endpoint(provider: &Provider) -> Option<&Endpoint> {
    provider
        .endpoints
        .iter()
        .find(|endpoint| matches!(endpoint.protocol, Protocol::AnthropicMessages))
}

/// Claude Code 的自定义 header 是单个环境变量：`name: value` 按 name 排序、
/// 以换行连接。宿主已按名排序，这里再排一次，不依赖上游顺序。
fn format_headers(headers: &[CustomHeader]) -> String {
    let mut sorted: Vec<&CustomHeader> = headers.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    sorted
        .iter()
        .map(|header| format!("{}: {}", header.name, header.value))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 候选清单（错误信息用）：零候选时说明原因，否则列出可选 slug。
fn candidate_hint(candidates: &[(&Provider, &Endpoint)]) -> String {
    if candidates.is_empty() {
        "当前没有可用的 anthropic-messages Provider".to_owned()
    } else {
        format!("可选：{}", slug_list(candidates))
    }
}

fn slug_list(candidates: &[(&Provider, &Endpoint)]) -> String {
    candidates
        .iter()
        .map(|(provider, _)| provider.slug.as_str())
        .collect::<Vec<_>>()
        .join("、")
}

/// 合并写入 settings.json：只增删本插件维护的四个键，其余键名与值逐字保留、
/// 相对键序不变（preserve_order）。无目标状态（没有键要写、也没有受管键要删）
/// 时不创建也不写，返回空路径列表。
fn write_settings(desired: &[(&str, String)]) -> Result<Vec<String>, String> {
    let path = Path::new(SETTINGS_FILE);
    let mut root = read_root(path)?;

    // env 缺失即补空对象（改的是内存里的副本；没有目标状态时不会落盘），
    // 存在但不是对象即失败——不静默重建，免得覆盖用户写坏的内容。
    if !root.contains_key("env") {
        root.insert(
            "env".to_owned(),
            serde_json::Value::Object(Default::default()),
        );
    }

    // env 的借用收在这个块里：下面 env 为空时要把它整个从 root 摘掉。
    let env_is_empty = {
        let Some(serde_json::Value::Object(env)) = root.get_mut("env") else {
            return Err("settings.json 的 env 不是 JSON 对象".to_owned());
        };
        let has_managed = MANAGED_KEYS.iter().any(|key| env.contains_key(*key));
        if desired.is_empty() && !has_managed {
            return Ok(Vec::new());
        }
        for key in MANAGED_KEYS {
            match desired.iter().find(|(managed, _)| *managed == key) {
                // 已存在的键原地替换（位置不变），新键追加。
                Some((_, value)) => {
                    env.insert(key.to_owned(), value.as_str().into());
                }
                // shift_remove 而非 remove：preserve_order 下 remove 是 swap_remove，
                // 会把末位键换到被删处，打乱其余键的相对顺序。
                None => {
                    env.shift_remove(key);
                }
            }
        }
        env.is_empty()
    };
    // 移除四个键后 env 为空时连 env 一并删掉；同样用 shift_remove 保住其余键序。
    if env_is_empty {
        root.shift_remove("env");
    }

    let mut json = serde_json::to_string_pretty(&serde_json::Value::Object(root))
        .map_err(|e| format!("序列化 settings.json 失败\n原因：{e}"))?;
    json.push('\n');

    // 原子写：先写同目录临时文件，成功后再改名替换。
    let tmp = Path::new(SETTINGS_TMP);
    fs::write(tmp, &json).map_err(|e| format!("写入 settings.json 临时文件失败\n原因：{e}"))?;
    fs::rename(tmp, path).map_err(|e| format!("替换 settings.json 失败\n原因：{e}"))?;

    Ok(vec![SETTINGS_FILE.to_owned()])
}

/// 读入现有 settings.json 的根对象：文件不存在视为空对象（新建时也不预建目录，
/// config_dir 由宿主解析时已创建）。非法 JSON、根不是对象即报错，不碰文件。
fn read_root(path: &Path) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Default::default()),
        Err(e) => return Err(format!("读取 settings.json 失败\n原因：{e}")),
    };
    let value: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("settings.json 不是合法 JSON\n原因：{e}"))?;
    match value {
        serde_json::Value::Object(root) => Ok(root),
        _ => Err("settings.json 的根不是 JSON 对象".to_owned()),
    }
}

export!(ClaudeCodePlugin);
