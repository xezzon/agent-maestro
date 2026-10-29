//! 配置存储的单元测试：读写、落盘往返、校验与保护态。

use super::*;
use crate::errors::{SecretCause, SecretError};
use crate::logging::capture::capture_window;
use crate::provider::{ModelEntry, Protocol};
use crate::secret::{SecretOp, SecretRef};
use crate::store::testutil::Fixture;

#[test]
fn empty_config_serializes_to_version_2_schema() {
    let config = Config::default();

    let text = serde_json::to_string(&config).unwrap();

    assert_eq!(
        text, r#"{"version":2,"providers":{},"variables":{}}"#,
        "空变量表恒序列化为 {{}}"
    );
    let parsed: Config = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed, config);
}

#[test]
fn config_parses_version_1_document() {
    let text = r#"{
            "version": 1,
            "providers": {
                "ollama": {
                    "base_url": {
                        "openai-completions": "http://localhost:11434/v1"
                    },
                    "models": [
                        { "id": "deepseek-chat", "display_name": "DeepSeek Chat" },
                        { "id": "deepseek-reasoner", "display_name": null }
                    ]
                }
            }
        }"#;

    let parsed: Config = serde_json::from_str(text).unwrap();

    let provider = &parsed.providers["ollama"];
    assert_eq!(
        provider
            .base_url
            .get(&Protocol::OpenaiCompletions)
            .map(String::as_str),
        Some("http://localhost:11434/v1")
    );
    assert!(!provider.base_url.contains_key(&Protocol::AnthropicMessages));
    assert!(!provider.api_key.is_set(), "api_key 未设置");
    assert_eq!(provider.models.len(), 2);
    assert_eq!(provider.models[0].id, "deepseek-chat");
    assert_eq!(
        provider.models[0].display_name.as_deref(),
        Some("DeepSeek Chat")
    );
    assert_eq!(provider.models[1].id, "deepseek-reasoner");
    assert_eq!(provider.models[1].display_name, None);
}

#[test]
fn missing_file_opens_as_empty_config() {
    let fixture = Fixture::new();
    let store = fixture.store();

    assert_eq!(store.get().unwrap(), &Config::default());
    assert!(store.get().unwrap().providers.is_empty());
}

#[test]
fn create_provider_persists_and_round_trips_through_disk() {
    let fixture = Fixture::new();
    let mut store = fixture.store();

    store
        .create_provider(
            "ollama",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "http://localhost:11434/v1".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap();

    let reopened = fixture.store();
    let provider = &reopened.get().unwrap().providers["ollama"];
    assert_eq!(
        provider
            .base_url
            .get(&Protocol::OpenaiCompletions)
            .map(String::as_str),
        Some("http://localhost:11434/v1")
    );
    assert!(!provider.base_url.contains_key(&Protocol::AnthropicMessages));
    assert!(!provider.api_key.is_set(), "api_key 未设置");
    assert!(provider.models.is_empty());
}

#[test]
fn update_provider_persists_and_round_trips_through_disk() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store
        .create_provider(
            "ollama",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "http://localhost:11434/v1".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap();

    store
        .update_provider(
            "ollama",
            ProviderWrite::overwrite(
                Provider {
                    base_url: BTreeMap::from([(
                        Protocol::OpenaiCompletions,
                        "https://api.example.com/v1".to_owned(),
                    )]),
                    ..Provider::default()
                },
                "sk-test".to_owned(),
            ),
        )
        .unwrap();

    let reopened = fixture.store();
    let provider = &reopened.get().unwrap().providers["ollama"];
    assert_eq!(
        provider
            .base_url
            .get(&Protocol::OpenaiCompletions)
            .map(String::as_str),
        Some("https://api.example.com/v1")
    );
    assert!(!provider.base_url.contains_key(&Protocol::AnthropicMessages));
    assert_eq!(
        provider.api_key.reference().map(SecretRef::slug),
        Some("ollama"),
        "落盘的是规范引用，真值在密钥链里"
    );
    assert!(provider.models.is_empty());
}

#[test]
fn update_provider_missing_slug_is_rejected_without_upsert() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    let mut store = fixture.store();

    let err = store
        .update_provider(
            "ghost",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "http://localhost:9".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap_err();
    assert!(matches!(err, ProviderError::MissingSlug { .. }));
    assert!(err.to_string().contains("ghost"));
    assert!(store.get().unwrap().providers.is_empty());
    assert!(!paths.config_path().exists(), "报错路径不得静默写入文件");
}

/// 整包替换：凭证随整包替换覆盖——真值写进密钥链，配置里只留规范引用。
#[test]
fn update_provider_replaces_api_key_with_whole_record() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store
        .create_provider(
            "ollama",
            ProviderWrite::overwrite(
                Provider {
                    base_url: BTreeMap::from([(
                        Protocol::OpenaiCompletions,
                        "http://localhost:11434/v1".to_owned(),
                    )]),
                    models: vec![ModelEntry {
                        id: "old-model".to_owned(),
                        display_name: Option::None,
                        ..ModelEntry::default()
                    }],
                    ..Provider::default()
                },
                "sk-old".to_owned(),
            ),
        )
        .unwrap();

    // 整包替换：api_key 携带新值即覆盖（密钥链里的值随之换新）。
    store
        .update_provider(
            "ollama",
            ProviderWrite::overwrite(
                Provider {
                    base_url: BTreeMap::from([(
                        Protocol::OpenaiCompletions,
                        "https://api.example.com/v1".to_owned(),
                    )]),
                    models: vec![ModelEntry {
                        id: "new-model".to_owned(),
                        display_name: Option::None,
                        ..ModelEntry::default()
                    }],
                    ..Provider::default()
                },
                "sk-new".to_owned(),
            ),
        )
        .unwrap();
    let provider = &store.get().unwrap().providers["ollama"];
    assert_eq!(
        provider.api_key.reference().map(SecretRef::slug),
        Some("ollama"),
        "api_key 随整包替换覆盖为规范引用"
    );
    assert_eq!(
        fixture
            .secrets()
            .entry(&SecretRef::for_provider("ollama").unwrap()),
        Some("sk-new".to_owned()),
        "真值在密钥链里"
    );
    assert_eq!(
        provider
            .base_url
            .get(&Protocol::OpenaiCompletions)
            .map(String::as_str),
        Some("https://api.example.com/v1"),
        "端点整包替换"
    );
    assert_eq!(provider.models[0].id, "new-model", "模型列表整包替换");

    // 清除：先删条目，再落空串。
    store
        .update_provider("ollama", ProviderWrite::clear(provider.clone()))
        .unwrap();
    assert!(
        !store.get().unwrap().providers["ollama"].api_key.is_set(),
        "清除后凭证未设置"
    );
    assert_eq!(
        fixture
            .secrets()
            .entry(&SecretRef::for_provider("ollama").unwrap()),
        None,
        "清除后密钥链条目一并删掉"
    );

    let reopened = fixture.store();
    assert!(
        !reopened.get().unwrap().providers["ollama"].api_key.is_set(),
        "api_key 未设置"
    );
}

#[test]
fn delete_provider_removes_record_with_models_and_api_key() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    fs::create_dir_all(paths.maestro_dir()).unwrap();
    fs::write(
        paths.config_path(),
        r#"{
                "version": 1,
                "providers": {
                    "openrouter": {
                        "base_url": {
                            "openai-completions": "https://api.example.com/v1"
                        },
                        "api_key": "sk-live",
                        "models": [
                            { "id": "z-model", "display_name": null },
                            { "id": "a-model", "display_name": "A Model" }
                        ]
                    }
                }
            }"#,
    )
    .unwrap();
    let mut store = fixture.store();

    store.delete_provider("openrouter").unwrap();

    let reopened = fixture.store();
    assert!(
        reopened.get().unwrap().providers.is_empty(),
        "删除后不留孤儿数据：端点、模型与 API Key 随记录一并移除"
    );
}

#[test]
fn delete_provider_missing_slug_is_rejected() {
    let fixture = Fixture::new();
    let mut store = fixture.store();

    let err = store.delete_provider("ghost").unwrap_err();

    assert!(matches!(err, ProviderError::MissingSlug { .. }));
    assert!(err.to_string().contains("ghost"));
}

#[test]
fn deleted_slug_can_be_recreated() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store
        .create_provider(
            "ollama",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "http://localhost:11434/v1".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap();

    store.delete_provider("ollama").unwrap();
    store
        .create_provider(
            "ollama",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::AnthropicMessages,
                    "http://127.0.0.1:8080".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap();

    let reopened = fixture.store();
    let provider = &reopened.get().unwrap().providers["ollama"];
    assert_eq!(
        provider
            .base_url
            .get(&Protocol::AnthropicMessages)
            .map(String::as_str),
        Some("http://127.0.0.1:8080")
    );
    assert!(!provider.base_url.contains_key(&Protocol::OpenaiCompletions));
    assert!(provider.models.is_empty());
}

#[test]
fn persisted_file_omits_unconfigured_protocol_slots() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    let mut store = fixture.store();

    store
        .create_provider(
            "ollama",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "http://localhost:11434/v1".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap();

    let text = fs::read_to_string(paths.config_path()).unwrap();
    assert!(text.contains(r#""openai-completions""#));
    assert!(
        !text.contains(r#""anthropic-messages""#),
        "未配置的协议槽不得写入文件（ADR 0003：键缺省而非空串）"
    );
}

#[test]
fn providers_are_written_sorted_by_slug() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    let mut store = fixture.store();

    for slug in ["zeta", "alpha", "midway"] {
        store
            .create_provider(
                slug,
                ProviderWrite::unchanged(Provider {
                    base_url: BTreeMap::from([(
                        Protocol::OpenaiCompletions,
                        "http://localhost:9/v1".to_owned(),
                    )]),
                    ..Provider::default()
                }),
            )
            .unwrap();
    }

    let text = fs::read_to_string(paths.config_path()).unwrap();
    let alpha = text.find("\"alpha\"").unwrap();
    let midway = text.find("\"midway\"").unwrap();
    let zeta = text.find("\"zeta\"").unwrap();
    assert!(alpha < midway && midway < zeta);
}

#[test]
fn multiple_providers_round_trip_through_disk() {
    let fixture = Fixture::new();
    let mut store = fixture.store();

    store
        .create_provider(
            "ollama",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "http://localhost:11434/v1".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap();
    store
        .create_provider(
            "openrouter",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::AnthropicMessages,
                    "https://anthropic.example.com/v1".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap();

    let reopened = fixture.store();
    let config = reopened.get().unwrap();
    assert_eq!(config.providers.len(), 2);
    assert_eq!(
        config.providers["ollama"]
            .base_url
            .get(&Protocol::OpenaiCompletions)
            .map(String::as_str),
        Some("http://localhost:11434/v1")
    );
    assert_eq!(
        config.providers["openrouter"]
            .base_url
            .get(&Protocol::AnthropicMessages)
            .map(String::as_str),
        Some("https://anthropic.example.com/v1")
    );
}

#[test]
fn create_provider_preserves_other_providers_models_and_endpoints() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    fs::create_dir_all(paths.maestro_dir()).unwrap();
    fs::write(
        paths.config_path(),
        r#"{
                "version": 1,
                "providers": {
                    "openrouter": {
                        "base_url": {
                            "openai-completions": "https://api.example.com/v1",
                            "anthropic-messages": "https://anthropic.example.com/v1"
                        },
                        "api_key": "sk-live",
                        "models": [
                            { "id": "z-model", "display_name": null },
                            { "id": "a-model", "display_name": "A Model" }
                        ]
                    }
                }
            }"#,
    )
    .unwrap();
    let mut store = fixture.store();

    store
        .create_provider(
            "ollama",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "http://localhost:11434/v1".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap();

    let reopened = fixture.store();
    let openrouter = &reopened.get().unwrap().providers["openrouter"];
    assert_eq!(
        openrouter.api_key.reference().map(SecretRef::slug),
        Some("openrouter"),
        "旧配置里的明文在启动时迁成了引用"
    );
    assert_eq!(
        fixture.secrets().entry(&reference("openrouter")),
        Some("sk-live".to_owned()),
        "真值搬进了密钥链"
    );
    assert_eq!(openrouter.models.len(), 2);
    assert_eq!(openrouter.models[0].id, "z-model");
    assert_eq!(openrouter.models[1].id, "a-model");
    assert_eq!(
        openrouter
            .base_url
            .get(&Protocol::OpenaiCompletions)
            .map(String::as_str),
        Some("https://api.example.com/v1")
    );
    assert_eq!(
        openrouter
            .base_url
            .get(&Protocol::AnthropicMessages)
            .map(String::as_str),
        Some("https://anthropic.example.com/v1")
    );
}

#[test]
fn corrupt_file_reports_error_with_path_and_refuses_writes() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    fs::create_dir_all(paths.maestro_dir()).unwrap();
    fs::write(paths.config_path(), "不是 JSON {{{").unwrap();
    let original = fs::read_to_string(paths.config_path()).unwrap();

    let mut store = fixture.store();

    let err = store.get().unwrap_err();
    assert!(matches!(err, ConfigError::Corrupt { .. }));
    assert!(
        err.to_string()
            .contains(paths.config_path().to_str().unwrap())
    );

    assert!(
        store
            .create_provider(
                "foo",
                ProviderWrite::unchanged(Provider {
                    base_url: BTreeMap::from([(
                        Protocol::OpenaiCompletions,
                        "http://localhost:9".to_owned(),
                    )]),
                    ..Provider::default()
                })
            )
            .is_err()
    );
    assert_eq!(fs::read_to_string(paths.config_path()).unwrap(), original);
}

#[test]
fn unsupported_version_reports_error_with_path_and_refuses_writes() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    fs::create_dir_all(paths.maestro_dir()).unwrap();
    fs::write(paths.config_path(), r#"{"version":99,"providers":{}}"#).unwrap();
    let original = fs::read_to_string(paths.config_path()).unwrap();

    let mut store = fixture.store();

    let err = store.get().unwrap_err();
    assert!(matches!(err, ConfigError::UnsupportedVersion { .. }));
    assert!(
        err.to_string()
            .contains(paths.config_path().to_str().unwrap())
    );

    assert!(
        store
            .create_provider(
                "foo",
                ProviderWrite::unchanged(Provider {
                    base_url: BTreeMap::from([(
                        Protocol::OpenaiCompletions,
                        "http://localhost:9".to_owned(),
                    )]),
                    ..Provider::default()
                })
            )
            .is_err()
    );
    assert_eq!(fs::read_to_string(paths.config_path()).unwrap(), original);
}

#[test]
fn duplicate_slug_is_rejected_and_original_kept() {
    let fixture = Fixture::new();
    let mut store = fixture.store();

    store
        .create_provider(
            "foo",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "http://localhost:9/v1".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap();

    let err = store
        .create_provider(
            "foo",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::AnthropicMessages,
                    "http://localhost:10".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap_err();
    assert!(matches!(err, ProviderError::DuplicateSlug { .. }));

    let config = store.get().unwrap();
    assert_eq!(config.providers.len(), 1);
    assert_eq!(
        config.providers["foo"]
            .base_url
            .get(&Protocol::OpenaiCompletions)
            .map(String::as_str),
        Some("http://localhost:9/v1")
    );
    assert!(
        !config.providers["foo"]
            .base_url
            .contains_key(&Protocol::AnthropicMessages)
    );
}

#[test]
fn create_provider_creates_missing_directories() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    let mut store = fixture.store();

    store
        .create_provider(
            "foo",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::AnthropicMessages,
                    "http://127.0.0.1:8080".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap();

    assert!(paths.config_path().exists());
}

#[test]
fn provider_with_models_round_trips_through_disk_order_preserved() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    let models = vec![
        ModelEntry {
            id: "accounts/fireworks/models/llama3.1".to_owned(),
            display_name: Option::None,
            ..ModelEntry::default()
        },
        ModelEntry {
            id: "Z-model".to_owned(),
            display_name: Option::Some("Z Model".to_owned()),
            ..ModelEntry::default()
        },
        ModelEntry {
            id: "gpt-4o".to_owned(),
            display_name: Option::None,
            ..ModelEntry::default()
        },
    ];

    store
        .create_provider(
            "openrouter",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "https://api.example.com/v1".to_owned(),
                )]),
                models: models.clone(),
                ..Provider::default()
            }),
        )
        .unwrap();

    let reopened = fixture.store();
    let provider = &reopened.get().unwrap().providers["openrouter"];
    assert_eq!(
        provider.models, models,
        "含模型的 Provider 保存/加载往返一致、保序，display_name 允许为 null"
    );
}

#[test]
fn same_model_id_can_exist_in_different_providers() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    let gpt_4o = || ModelEntry {
        id: "gpt-4o".to_owned(),
        display_name: Option::None,
        ..ModelEntry::default()
    };

    store
        .create_provider(
            "openai",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "https://api.openai.com/v1".to_owned(),
                )]),
                models: vec![gpt_4o()],
                ..Provider::default()
            }),
        )
        .unwrap();
    store
        .create_provider(
            "gateway",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "http://127.0.0.1:8080/v1".to_owned(),
                )]),
                models: vec![
                    gpt_4o(),
                    ModelEntry {
                        id: "gpt-4o-mini".to_owned(),
                        display_name: Option::None,
                        ..ModelEntry::default()
                    },
                ],
                ..Provider::default()
            }),
        )
        .unwrap();

    let reopened = fixture.store();
    let config = reopened.get().unwrap();
    assert_eq!(config.providers["openai"].models, vec![gpt_4o()]);
    assert_eq!(
        config.providers["gateway"].models,
        vec![
            gpt_4o(),
            ModelEntry {
                id: "gpt-4o-mini".to_owned(),
                display_name: Option::None,
                ..ModelEntry::default()
            }
        ]
    );
}

#[test]
fn store_does_not_validate_model_ids() {
    // 业务校验（非空、Provider 内唯一）由前端内联完成；存储层原样保存，不拦不补。
    let fixture = Fixture::new();
    let mut store = fixture.store();

    store
        .create_provider(
            "foo",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "http://localhost:9/v1".to_owned(),
                )]),
                models: vec![
                    ModelEntry {
                        id: "gpt-4o".to_owned(),
                        display_name: Option::None,
                        ..ModelEntry::default()
                    },
                    ModelEntry {
                        id: "GPT-4O".to_owned(),
                        display_name: Option::None,
                        ..ModelEntry::default()
                    },
                    ModelEntry {
                        id: String::new(),
                        display_name: Option::None,
                        ..ModelEntry::default()
                    },
                ],
                ..Provider::default()
            }),
        )
        .unwrap();

    let reopened = fixture.store();
    let models = &reopened.get().unwrap().providers["foo"].models;
    assert_eq!(models.len(), 3);
    assert_eq!(models[0].id, "gpt-4o");
    assert_eq!(models[1].id, "GPT-4O", "大小写敏感：大小写变体可并存");
    assert_eq!(models[2].id, "", "空 ID 同样不被存储层拦截");
}

#[test]
fn plugins_section_defaults_to_empty_for_legacy_config_files() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    fs::create_dir_all(paths.maestro_dir()).unwrap();
    fs::write(paths.config_path(), r#"{"version":1,"providers":{}}"#).unwrap();

    let store = fixture.store();

    assert!(
        store.get().unwrap().plugins.is_empty(),
        "缺 plugins 段的旧配置文件直接可用（纯增量字段，见 issue #34）"
    );
}

/// 全局变量表是纯增量字段（ADR 0015）：无 variables 的旧文件照常读入得空表，
/// CONFIG_VERSION 保持 1；空表照常写出为 `{}`。
#[test]
fn variables_default_to_empty_and_serialize_as_an_empty_object() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    fs::create_dir_all(paths.maestro_dir()).unwrap();
    fs::write(paths.config_path(), r#"{"version":1,"providers":{}}"#).unwrap();

    let store = fixture.store();

    assert!(store.get().unwrap().variables.is_empty());
    assert_eq!(
        serde_json::to_string(store.get().unwrap()).unwrap(),
        r#"{"version":1,"providers":{},"variables":{}}"#,
        "空变量表序列化为 {{}}，不做省略"
    );
}

#[test]
fn variables_round_trip_through_disk() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let variables = BTreeMap::from([
        ("HOST".to_owned(), "api.example.com".to_owned()),
        ("EMPTY".to_owned(), String::new()),
    ]);
    {
        let mut config = store.get().unwrap().clone();
        config.variables = variables.clone();
        store.persist(&mut config).unwrap();
    }

    let reopened = fixture.store();

    assert_eq!(reopened.get().unwrap().variables, variables);
}

/// 整包替换变量表：新增、修改与删除都走同一条全量保存路径（issue #90），
/// 成功后原子写回磁盘。
#[test]
fn set_variables_replaces_whole_table_and_round_trips_through_disk() {
    let fixture = Fixture::new();
    let mut store = fixture.store();

    store
        .set_variables(BTreeMap::from([
            ("HOST".to_owned(), "api.example.com".to_owned()),
            ("TOKEN".to_owned(), "old".to_owned()),
        ]))
        .unwrap();
    // 第二次保存携带值修改后的表：整包替换，不与旧表合并。
    store
        .set_variables(BTreeMap::from([
            ("HOST".to_owned(), "gateway.example.com".to_owned()),
            ("TOKEN".to_owned(), "old".to_owned()),
        ]))
        .unwrap();

    let reopened = fixture.store();
    assert_eq!(
        reopened.get().unwrap().variables,
        BTreeMap::from([
            ("HOST".to_owned(), "gateway.example.com".to_owned()),
            ("TOKEN".to_owned(), "old".to_owned())
        ])
    );
}

/// 变量表清空同样是合法保存：空表照常写出为 `{}`。
#[test]
fn set_variables_to_empty_clears_the_table() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    let mut store = fixture.store();
    store
        .set_variables(BTreeMap::from([(
            "HOST".to_owned(),
            "api.example.com".to_owned(),
        )]))
        .unwrap();

    store.set_variables(BTreeMap::new()).unwrap();

    assert!(store.get().unwrap().variables.is_empty());
    let reopened = fixture.store();
    assert!(reopened.get().unwrap().variables.is_empty());
    assert_eq!(
        fs::read_to_string(paths.config_path()).unwrap(),
        "{\n  \"version\": 2,\n  \"providers\": {},\n  \"variables\": {}\n}\n"
    );
}

/// L1 红线（ADR 0008）：Config 的派生 Debug 内层调用 Provider 的手工 Debug，
/// 嵌套渲染同样不得携带 api_key 明文。
///
/// 启动迁移成功后配置里只剩引用：本用例钉住引用形态下的脱敏；明文形态
/// （迁移失败、明文仍留在配置里）由下一条用例覆盖。
#[test]
fn config_debug_output_never_contains_the_api_key() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"{
            "version": 1,
            "providers": {
                "openrouter": { "api_key": "sk-config-canary-4b7e" }
            }
        }"#,
    );
    let store = fixture.store();

    assert!(
        store.get().unwrap().providers["openrouter"]
            .api_key
            .reference()
            .is_some(),
        "前提：启动迁移已把明文换成引用"
    );
    let rendered = format!("{:?}", store.get().unwrap());

    assert!(
        !rendered.contains("sk-config-canary-4b7e"),
        "Config 的 Debug 输出不得包含 api_key：{rendered}"
    );
}

/// 密钥链写入失败时迁移保留明文：这时内存里确实攥着明文，Debug 脱敏是那
/// 时唯一的防线（ADR 0008）。
#[test]
fn config_debug_output_never_contains_plaintext_when_the_migration_write_fails() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"{
            "version": 1,
            "providers": {
                "openrouter": { "api_key": "sk-config-canary-plain" }
            }
        }"#,
    );
    fixture
        .secrets()
        .fail_next(SecretOp::Write, unavailable(SecretOp::Write));
    let store = fixture.store();

    assert_eq!(
        store.get().unwrap().providers["openrouter"]
            .api_key
            .plaintext(),
        Some("sk-config-canary-plain"),
        "前提：写入失败，明文仍留在配置里"
    );
    let rendered = format!("{:?}", store.get().unwrap());

    assert!(
        !rendered.contains("sk-config-canary-plain"),
        "Config 的 Debug 输出不得包含 api_key：{rendered}"
    );
}

#[test]
fn corrupt_store_refuses_plugin_writes() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    fs::create_dir_all(paths.maestro_dir()).unwrap();
    fs::write(paths.config_path(), "不是 JSON {{{").unwrap();
    let mut store = fixture.store();
    let entry = PluginEntry {
        source: "https://example.com/manifest.json".to_owned(),
        enabled: true,
        id: "pi".to_owned(),
    };

    assert!(store.set_plugin_enabled("builtin:pi", true).is_err());
    assert!(store.add_plugin(&entry).is_err());
    assert!(store.delete_plugin("builtin:pi").is_err());
    assert!(store.set_variables(BTreeMap::new()).is_err());
}

/// 变量名校验先于存储状态：配置损坏与非法名同时出现时报非法名（不被保护态
/// 抢先复述成 `Corrupt`），且损坏的原文件不被改写。
#[test]
fn invalid_variable_names_win_over_a_corrupt_config() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    let config_path = paths.config_path();
    fs::create_dir_all(paths.maestro_dir()).unwrap();
    fs::write(&config_path, "不是 JSON {{{").unwrap();
    let mut store = fixture.store();

    let err = store
        .set_variables(BTreeMap::from([("a-b".to_owned(), "x".to_owned())]))
        .unwrap_err();

    assert!(
        matches!(err, VariableError::InvalidVariableName { .. }),
        "非法名先于保护态：{err:?}"
    );
    assert_eq!(fs::read_to_string(&config_path).unwrap(), "不是 JSON {{{");
}

#[test]
fn create_provider_rejects_references_to_undefined_variables() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    let mut store = fixture.store();

    let err = store
        .create_provider(
            "gateway",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "https://${HOST}/v1".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap_err();

    assert!(matches!(err, ProviderError::Interpolation(_)));
    let message = err.to_string();
    assert!(message.contains("HOST"), "{message}");
    assert!(message.contains("base_url.openai-completions"), "{message}");
    assert!(store.get().unwrap().providers.is_empty());
    assert!(!paths.config_path().exists(), "报错路径不得静默写入文件");
}

/// 落盘前拦截与投影时拦截逐字同报：同一份配置、同一个原因，两条路径给出
/// 完全相同的一句文案（ADR 0015「报错与投影时同因同报」；共享构造函数见
/// issue #128）。逐字断言本体住在 `errors::ProviderInterpolationError`
/// （句式的家），这里钉的是 **两条路径汇入同一句式**。
#[test]
fn store_and_projection_report_interpolation_failures_identically() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    let provider = Provider {
        base_url: BTreeMap::from([(Protocol::OpenaiCompletions, "https://${HOST}/v1".to_owned())]),
        ..Provider::default()
    };
    let mut providers = BTreeMap::new();
    providers.insert("gateway".to_owned(), provider.clone());

    let stored = store
        .create_provider("gateway", ProviderWrite::unchanged(provider))
        .unwrap_err()
        .to_string();
    let projected = crate::provider::interpolate_providers(&providers, &Variables::new())
        .unwrap_err()
        .to_string();

    assert_eq!(
        stored, projected,
        "落盘前拦截与投影时拦截逐字同报：{stored}"
    );
}

/// 已定义的变量与带内联默认值的引用都可通过；存储层原样保存，插值发生在投影时。
#[test]
fn create_provider_accepts_defined_variables_and_inline_defaults() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store
        .set_variables(BTreeMap::from([(
            "HOST".to_owned(),
            "api.example.com".to_owned(),
        )]))
        .unwrap();

    store
        .create_provider(
            "gateway",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([
                    (Protocol::OpenaiCompletions, "https://${HOST}/v1".to_owned()),
                    (
                        Protocol::AnthropicMessages,
                        "https://${FALLBACK:host.example.com}".to_owned(),
                    ),
                ]),
                ..Provider::default()
            }),
        )
        .unwrap();

    let reopened = fixture.store();
    assert_eq!(
        reopened.get().unwrap().providers["gateway"]
            .base_url
            .get(&Protocol::OpenaiCompletions)
            .map(String::as_str),
        Some("https://${HOST}/v1"),
    );
}

#[test]
fn update_provider_rejects_undefined_variables_and_keeps_the_original() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store
        .create_provider(
            "gateway",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "https://api.example.com/v1".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap();

    let err = store
        .update_provider(
            "gateway",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "https://${MISSING}/v1".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap_err();

    assert!(matches!(err, ProviderError::Interpolation(_)));
    let reopened = fixture.store();
    assert_eq!(
        reopened.get().unwrap().providers["gateway"]
            .base_url
            .get(&Protocol::OpenaiCompletions)
            .map(String::as_str),
        Some("https://api.example.com/v1"),
        "被拒的更新不落盘，原记录保持不变"
    );
}

/// 删除在用变量整表拒绝：报错与投影时插值失败同款文案，落盘不变。
#[test]
fn set_variables_rejects_removing_a_variable_in_use() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store
        .set_variables(BTreeMap::from([
            ("HOST".to_owned(), "api.example.com".to_owned()),
            ("UNUSED".to_owned(), "x".to_owned()),
        ]))
        .unwrap();
    store
        .create_provider(
            "gateway",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "https://${HOST}/v1".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap();

    let err = store
        .set_variables(BTreeMap::from([("UNUSED".to_owned(), "x".to_owned())]))
        .unwrap_err();

    assert!(matches!(err, VariableError::Interpolation(_)));
    let message = err.to_string();
    assert!(message.contains("HOST"), "{message}");
    assert!(message.contains("gateway"), "{message}");
    let reopened = fixture.store();
    assert!(
        reopened.get().unwrap().variables.contains_key("HOST"),
        "被拒的保存不落盘，变量表保持不变"
    );
}

/// 只被带内联默认值的引用提到的变量可以删除（删除后插值仍可解析）。
#[test]
fn set_variables_allows_removing_a_variable_only_referenced_with_inline_default() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store
        .set_variables(BTreeMap::from([("OPT".to_owned(), "x".to_owned())]))
        .unwrap();
    store
        .create_provider(
            "gateway",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "https://${OPT:fallback.example.com}/v1".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap();

    store.set_variables(BTreeMap::new()).unwrap();

    assert!(store.get().unwrap().variables.is_empty());
}

/// 改值与新增不受一致性约束：在用变量的值可以修改。
#[test]
fn set_variables_allows_changing_the_value_of_a_variable_in_use() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store
        .set_variables(BTreeMap::from([(
            "HOST".to_owned(),
            "api.example.com".to_owned(),
        )]))
        .unwrap();
    store
        .create_provider(
            "gateway",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "https://${HOST}/v1".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap();

    store
        .set_variables(BTreeMap::from([(
            "HOST".to_owned(),
            "gateway.example.com".to_owned(),
        )]))
        .unwrap();

    assert_eq!(
        store.get().unwrap().variables["HOST"],
        "gateway.example.com"
    );
}

/// 占位符语法错误同样在落盘前拦截：与投影时插值失败同因同报。
#[test]
fn create_provider_rejects_malformed_placeholders() {
    let fixture = Fixture::new();
    let mut store = fixture.store();

    let err = store
        .create_provider(
            "gateway",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "https://${HOST/v1".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap_err();

    assert!(matches!(err, ProviderError::Interpolation(_)));
    assert!(err.to_string().contains("base_url.openai-completions"));
    assert!(store.get().unwrap().providers.is_empty());
}

/// 手工改出的非法模板同样让变量表保存被拒：落盘不变式＝全部 Provider 对
/// 当前（变更后的）变量表可插值。
#[test]
fn set_variables_rejected_when_a_provider_has_a_malformed_template() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    fs::create_dir_all(paths.maestro_dir()).unwrap();
    fs::write(
        paths.config_path(),
        r#"{
                "version": 1,
                "providers": {
                    "gateway": {
                        "base_url": {
                            "openai-completions": "https://${HOST/v1"
                        }
                    }
                },
                "variables": {"HOST": "api.example.com"}
            }"#,
    )
    .unwrap();
    let original = fs::read_to_string(paths.config_path()).unwrap();
    let mut store = fixture.store();

    let err = store
        .set_variables(BTreeMap::from([(
            "HOST".to_owned(),
            "gateway.example.com".to_owned(),
        )]))
        .unwrap_err();

    assert!(matches!(err, VariableError::Interpolation(_)));
    assert_eq!(
        fs::read_to_string(paths.config_path()).unwrap(),
        original,
        "被拒的保存不落盘"
    );
}

/// 变量名合法性在后端同样强制（ADR 0014 的 `[A-Za-z0-9_]+`）：绕过前端
/// 直传命令的非法键名整表拒绝、不落盘。
#[test]
fn set_variables_rejects_invalid_variable_names() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    let mut store = fixture.store();

    for name in ["a-b", "", "带 空格"] {
        let err = store
            .set_variables(BTreeMap::from([(name.to_owned(), "x".to_owned())]))
            .unwrap_err();

        assert!(
            matches!(err, VariableError::InvalidVariableName { .. }),
            "name={name:?}"
        );
        assert!(!paths.config_path().exists(), "报错路径不得落盘");
    }
}

/// 空默认值是合法状态（`${EMPTY}` 解析为空串即按未配置端点处理，ADR 0015），
/// 后端不拒绝；非空的合法名照常保存。
#[test]
fn set_variables_accepts_an_empty_default_value() {
    let fixture = Fixture::new();
    let mut store = fixture.store();

    store
        .set_variables(BTreeMap::from([
            ("EMPTY".to_owned(), String::new()),
            ("HOST".to_owned(), "api.example.com".to_owned()),
        ]))
        .unwrap();

    assert_eq!(store.get().unwrap().variables["EMPTY"], "");
    let reopened = fixture.store();
    assert_eq!(reopened.get().unwrap().variables["EMPTY"], "");
}

/// 旧配置文件缺 `enabled` 键：读入即启用；随后任一次落盘都补全该键，
/// 原有数据（端点/模型/api_key/变量）一字不丢。
#[test]
fn legacy_config_without_enabled_reads_as_enabled_and_round_trips() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    let path = paths.config_path();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        r#"{
                "version": 1,
                "providers": {
                    "ollama": {
                        "base_url": {"openai-completions": "http://localhost:11434/v1"},
                        "api_key": "sk-legacy",
                        "models": [{"id": "deepseek-chat", "display_name": "DeepSeek Chat"}]
                    }
                },
                "variables": {"HOST": "api.example.com"}
            }"#,
    )
    .unwrap();

    let mut store = fixture.store();
    assert!(
        store.get().unwrap().providers["ollama"].enabled,
        "旧配置缺 enabled 键即视为启用"
    );

    // 一次无关的落盘（新增变量）后，旧记录被完整保留并补全 enabled 键。
    let mut variables = store.get().unwrap().variables.clone();
    variables.insert("EXTRA".to_owned(), "1".to_owned());
    store.set_variables(variables).unwrap();

    let reopened = fixture.store();
    let provider = &reopened.get().unwrap().providers["ollama"];
    assert!(provider.enabled);
    assert_eq!(
        provider
            .base_url
            .get(&Protocol::OpenaiCompletions)
            .map(String::as_str),
        Some("http://localhost:11434/v1")
    );
    assert_eq!(
        provider.api_key.reference().map(SecretRef::slug),
        Some("ollama"),
        "旧配置里的明文在启动时迁成了引用"
    );
    assert_eq!(provider.models.len(), 1);
    assert_eq!(provider.models[0].id, "deepseek-chat");
    assert_eq!(
        provider.models[0].display_name.as_deref(),
        Some("DeepSeek Chat")
    );
    assert_eq!(reopened.get().unwrap().variables["HOST"], "api.example.com");
    assert_eq!(reopened.get().unwrap().variables["EXTRA"], "1");
}

/// 新字段随记录落盘并原样读回：custom_header / limit / capabilities 往返一致。
#[test]
fn provider_with_custom_header_limit_and_capabilities_round_trips_through_disk() {
    use crate::provider::{ModelCapability, ModelLimit};

    let fixture = Fixture::new();
    let mut store = fixture.store();
    let provider = Provider {
        base_url: BTreeMap::from([(
            Protocol::OpenaiCompletions,
            "https://api.example.com/v1".to_owned(),
        )]),
        custom_header: BTreeMap::from([("X-Gateway-Key".to_owned(), "gw-plain".to_owned())]),
        models: vec![ModelEntry {
            id: "gpt-4o".to_owned(),
            display_name: Some("GPT-4o".to_owned()),
            limit: ModelLimit {
                context_window: Some(128_000),
                max_input: None,
                max_output: Some(16_384),
            },
            capabilities: vec![ModelCapability::ToolUse, ModelCapability::ImageIn],
        }],
        ..Provider::default()
    };

    store
        .create_provider("gateway", ProviderWrite::unchanged(provider.clone()))
        .unwrap();

    let reopened = fixture.store();
    assert_eq!(
        reopened.get().unwrap().providers["gateway"],
        provider,
        "含新字段的 Provider 落盘后原样读回"
    );
}

/// 旧配置文件（无 custom_header / limit / capabilities）：读入即空映射、
/// 空 limit、空能力集合；随后任一次落盘补齐固定形状。
#[test]
fn legacy_config_without_new_fields_reads_as_unset_and_writes_fixed_shape() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    let path = paths.config_path();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        r#"{
                "version": 1,
                "providers": {
                    "ollama": {
                        "base_url": {"openai-completions": "http://localhost:11434/v1"},
                        "models": [{"id": "deepseek-chat", "display_name": "DeepSeek Chat"}]
                    }
                }
            }"#,
    )
    .unwrap();

    let mut store = fixture.store();
    let provider = &store.get().unwrap().providers["ollama"];
    assert!(provider.custom_header.is_empty(), "旧配置读作空映射");
    assert_eq!(
        provider.models[0].limit,
        crate::provider::ModelLimit::default()
    );
    assert!(provider.models[0].capabilities.is_empty());

    // 一次无关的落盘：新字段补齐固定形状，原数据一字不丢。
    let mut variables = store.get().unwrap().variables.clone();
    variables.insert("EXTRA".to_owned(), "1".to_owned());
    store.set_variables(variables).unwrap();

    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let provider = &written["providers"]["ollama"];
    assert_eq!(
        provider["custom_header"],
        serde_json::json!({}),
        "{written}"
    );
    let model = &provider["models"][0];
    assert_eq!(model["id"], "deepseek-chat");
    assert_eq!(model["limit"], serde_json::json!({}), "{written}");
    assert_eq!(model["capabilities"], serde_json::json!([]), "{written}");
}

#[test]
fn set_provider_enabled_toggles_and_round_trips_through_disk() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store
        .create_provider("ollama", ProviderWrite::unchanged(Provider::default()))
        .unwrap();

    store.set_provider_enabled("ollama", false).unwrap();

    assert!(!store.get().unwrap().providers["ollama"].enabled);
    assert!(
        !fixture.store().get().unwrap().providers["ollama"].enabled,
        "禁用态随记录落盘"
    );

    store.set_provider_enabled("ollama", true).unwrap();
    assert!(fixture.store().get().unwrap().providers["ollama"].enabled);
}

#[test]
fn set_provider_enabled_is_idempotent_and_missing_slug_is_rejected() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store
        .create_provider("ollama", ProviderWrite::unchanged(Provider::default()))
        .unwrap();

    store.set_provider_enabled("ollama", true).unwrap();
    assert!(store.get().unwrap().providers["ollama"].enabled);

    let err = store.set_provider_enabled("missing", false).unwrap_err();
    assert!(matches!(err, ProviderError::MissingSlug { .. }));
}

/// 幂等调用**不写盘**（不只是状态不变）。
///
/// 钉「写没写」而非「状态变没变」：先把磁盘内容手工改成与内存不同的样子
/// （`enabled` 翻成 false），再发一次状态未变的调用；若实现落了盘，文件会被
/// 改写成内存状态，断言随即失败。末尾再翻一次状态作对照，证明文件确实
/// 可被改写且翻转被记录——否则「文件没变」可能只是文件坏了。
#[test]
fn set_provider_enabled_does_not_write_to_disk_when_state_is_unchanged() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    let mut store = fixture.store();
    store
        .create_provider("ollama", ProviderWrite::unchanged(Provider::default()))
        .unwrap();
    let path = paths.config_path();

    // 手工把磁盘内容改成与内存不同的样子：ollama 在盘上是禁用态。
    let hand_edited = r#"{
  "version": 1,
  "providers": { "ollama": { "enabled": false } },
  "variables": {}
}
"#;
    fs::write(&path, hand_edited).unwrap();

    store.set_provider_enabled("ollama", true).unwrap();

    assert!(
        store.get().unwrap().providers["ollama"].enabled,
        "幂等调用后内存状态不变"
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        hand_edited,
        "状态未变的幂等调用不得写盘"
    );

    // 对照：状态确实变化时必须落盘，且翻转被记录。
    store.set_provider_enabled("ollama", false).unwrap();
    assert_ne!(
        fs::read_to_string(&path).unwrap(),
        hand_edited,
        "状态变化时必须写盘"
    );
    assert!(
        !fixture.store().get().unwrap().providers["ollama"].enabled,
        "翻转后的启用态必须落盘"
    );
}

/// 禁用的 Provider 同样受插值校验约束：落盘内容始终对变量表可解析，
/// 禁用只是投影时的结构过滤，不放宽 provider↔variables 一致性。
#[test]
fn disabled_provider_is_still_subject_to_interpolation_checks() {
    let fixture = Fixture::new();
    let mut store = fixture.store();

    // 禁用态也不能保存引用未定义变量的记录。
    let err = store
        .create_provider(
            "gateway",
            ProviderWrite::unchanged(Provider {
                enabled: false,
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "https://${UNDEFINED}/v1".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap_err();
    assert!(matches!(err, ProviderError::Interpolation(_)));

    // 先建一个引用 HOST 的启用 Provider，再禁用。
    store
        .set_variables(BTreeMap::from([(
            "HOST".to_owned(),
            "api.example.com".to_owned(),
        )]))
        .unwrap();
    store
        .create_provider(
            "enabled",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "https://${HOST}/v1".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap();
    store.set_provider_enabled("enabled", false).unwrap();

    // 禁用项引用的变量同样不能删除：禁用者仍算「在使用中」。
    let err = store.set_variables(BTreeMap::new()).unwrap_err();
    assert!(matches!(err, VariableError::Interpolation(_)));
    assert!(
        store.get().unwrap().variables.contains_key("HOST"),
        "被拒的删变量不改配置"
    );
}

/// 整包更新为禁用态同样要过插值校验：不能把记录改成引用未定义变量。
#[test]
fn update_provider_to_disabled_still_checks_interpolation() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store
        .create_provider("gateway", ProviderWrite::unchanged(Provider::default()))
        .unwrap();

    let err = store
        .update_provider(
            "gateway",
            ProviderWrite::unchanged(Provider {
                enabled: false,
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "https://${MISSING}/v1".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap_err();
    assert!(matches!(err, ProviderError::Interpolation(_)));

    let provider = &store.get().unwrap().providers["gateway"];
    assert!(provider.enabled, "被拒的更新不改配置");
    assert_ne!(
        provider
            .base_url
            .get(&Protocol::OpenaiCompletions)
            .map(String::as_str),
        Some("https://${MISSING}/v1")
    );
}

// ---------------------------------------------------------------------------
// 凭证三态与密钥链（ADR 0002）：真值只进密钥链，配置只留引用。
//
// 这一段钉的是**存储层接缝**：注入内存密钥链 + 临时配置文件，观察到的只有
// 「密钥链里有什么」「配置文件里有什么」「报错怎么说」——真值任何时候都不该
// 出现在配置或断言输出里。
// ---------------------------------------------------------------------------

/// 本 Provider 的规范引用（断言用）。
fn reference(slug: &str) -> SecretRef {
    SecretRef::for_provider(slug).unwrap()
}

/// 配置文件里的原始文本。
fn config_text(fixture: &Fixture) -> String {
    fs::read_to_string(fixture.paths().config_path()).unwrap()
}

/// 让落盘失败：把配置文件换成同名目录，rename 覆盖不了它。
///
/// 落盘失败是「配置写不下去」的唯一入口：密钥链写成功了，配置却没写下去——
/// 这时密钥链必须回滚到变更前的状态。
fn break_persist(fixture: &Fixture) {
    let path = fixture.paths().config_path();
    // 文件可能还不存在（首次保存前）：换成同名目录即可，rename 覆盖不了它。
    let _ = fs::remove_file(&path);
    fs::create_dir_all(&path).unwrap();
}

/// 让落盘失败：配置目录改成只读——配置照样读得到，临时文件却建不出来。
///
/// 「读得到、写不下去」是**启动迁移**落盘失败的入口：迁移读得到旧明文，
/// 写回引用时却写不下去（配置目录只读）。与 [`break_persist`] 的区别在于
/// 那个换掉的是配置文件本身，启动时连读都读不到。
///
/// 只在 Unix 上成立：Windows 的目录只读属性拦不住「在目录里建文件」，那边
/// 换用别的造法（本仓库的 CI 只跑 Linux）。以 root 运行时只读拦不住写入，
/// 落盘会照常成功——那条用例随即失败（而不是悄悄放过）。
///
/// 返回原权限：恢复时原样还回去，不猜一个模式。
#[cfg(unix)]
fn freeze_persist(fixture: &Fixture) -> fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    let path = fixture.paths().config_path();
    let dir = path.parent().unwrap();
    let original = fs::metadata(dir).unwrap().permissions();
    fs::set_permissions(dir, fs::Permissions::from_mode(0o555)).unwrap();
    original
}

/// 恢复可写：落盘重新可行（重启重试的前提）。
#[cfg(unix)]
fn thaw_persist(fixture: &Fixture, original: fs::Permissions) {
    let path = fixture.paths().config_path();
    let dir = path.parent().unwrap();
    fs::set_permissions(dir, original).unwrap();
}

/// 密钥链不可用（后端缺席 / 条目库锁着）。
///
/// 「哪个动作折成哪个失败变体」由 [`SecretOp::into_error`] 独家给出，替身与
/// 生产实现共用同一条规则。
fn unavailable(op: SecretOp) -> SecretError {
    op.into_error(SecretCause::Unavailable {
        reason: "密钥链被锁着".to_owned(),
    })
}

/// 覆盖：真值写进密钥链，配置里只留规范引用。
#[test]
fn overwrite_writes_the_key_to_the_keychain_and_keeps_only_a_reference() {
    let fixture = Fixture::new();
    let mut store = fixture.store();

    store
        .create_provider(
            "ollama",
            ProviderWrite::overwrite(Provider::default(), "sk-new".to_owned()),
        )
        .unwrap();

    assert_eq!(
        fixture.secrets().entries(),
        BTreeMap::from([("provider/ollama/api_key".to_owned(), "sk-new".to_owned())]),
        "真值在密钥链里，且只有这一条"
    );
    let provider = &store.get().unwrap().providers["ollama"];
    assert_eq!(
        provider.api_key.reference().map(SecretRef::slug),
        Some("ollama"),
        "配置里只留规范引用"
    );
    let text = config_text(&fixture);
    assert!(!text.contains("sk-new"), "落盘内容不得含真值：{text}");
    assert!(
        text.contains("secret://io.github.xezzon.agent-maestro/provider/ollama/api_key"),
        "落盘内容是规范引用：{text}"
    );
}

/// 新建缺省或清除：都落成未设置，且完全不碰密钥链。
#[test]
fn create_without_a_credential_leaves_the_field_unset_and_the_keychain_untouched() {
    let fixture = Fixture::new();
    let mut store = fixture.store();

    store
        .create_provider("ollama", ProviderWrite::unchanged(Provider::default()))
        .unwrap();
    store
        .create_provider("empty", ProviderWrite::clear(Provider::default()))
        .unwrap();

    let config = store.get().unwrap();
    assert!(!config.providers["ollama"].api_key.is_set());
    assert!(
        !config.providers["empty"].api_key.is_set(),
        "新建时清除同样落成未设置"
    );
    assert!(
        fixture.secrets().calls().is_empty(),
        "新建没有条目可写也没有条目可删：{:?}",
        fixture.secrets().calls()
    );
}

/// 更新时凭证不变：完全不碰密钥链，字段沿用旧值。
///
/// 规范引用回传走的就是这条路径（命令层把本 slug 的规范引用归一成「不变」），
/// 故前端拿到的引用原样送回既不产生新密钥、也不丢密钥。
#[test]
fn an_unchanged_credential_never_touches_the_keychain() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store
        .create_provider(
            "ollama",
            ProviderWrite::overwrite(Provider::default(), "sk-old".to_owned()),
        )
        .unwrap();
    let before = fixture.secrets().calls().len();

    store
        .update_provider(
            "ollama",
            ProviderWrite::unchanged(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "http://localhost:11434/v1".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap();

    assert_eq!(
        fixture.secrets().calls().len(),
        before,
        "凭证不变时密钥链一个动作都不该有：{:?}",
        fixture.secrets().calls()
    );
    assert_eq!(
        fixture.secrets().entry(&reference("ollama")),
        Some("sk-old".to_owned()),
        "旧值原样留着"
    );
    assert_eq!(
        store.get().unwrap().providers["ollama"]
            .api_key
            .reference()
            .map(SecretRef::slug),
        Some("ollama"),
        "字段沿用旧值"
    );
}

/// 清除：先删条目，再落空串。
#[test]
fn clearing_a_credential_deletes_the_entry_and_empties_the_field() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store
        .create_provider(
            "ollama",
            ProviderWrite::overwrite(Provider::default(), "sk-old".to_owned()),
        )
        .unwrap();

    store
        .update_provider("ollama", ProviderWrite::clear(Provider::default()))
        .unwrap();

    assert_eq!(
        fixture.secrets().entry(&reference("ollama")),
        None,
        "条目被删掉"
    );
    assert!(
        !store.get().unwrap().providers["ollama"].api_key.is_set(),
        "字段落成未设置"
    );
}

/// 清除时条目本就不存在：按幂等成功处理。
#[test]
fn clearing_a_missing_entry_is_idempotent() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"{
            "version": 2,
            "providers": {
                "ollama": {
                    "api_key": "secret://io.github.xezzon.agent-maestro/provider/ollama/api_key"
                }
            }
        }"#,
    );
    let mut store = fixture.store();

    store
        .update_provider("ollama", ProviderWrite::clear(Provider::default()))
        .unwrap();

    assert!(
        !store.get().unwrap().providers["ollama"].api_key.is_set(),
        "条目不存在也清得掉"
    );
}

/// 删除 Provider 只在字段为引用时级联删除条目；明文与未设置的记录没有
/// 条目可删，直接删记录、不碰密钥链。
///
/// 明文只在迁移失败时残留（#147）：密钥链写不进去，明文原样留在配置里。
#[test]
fn deleting_a_provider_cascades_to_the_keychain_only_for_a_reference() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"{
            "version": 1,
            "providers": {
                "legacy": { "api_key": "sk-legacy" },
                "unset": {}
            }
        }"#,
    );
    // 迁移写不进密钥链：legacy 的明文原样留在配置里，它没有条目可删。
    fixture
        .secrets()
        .fail_next(SecretOp::Write, unavailable(SecretOp::Write));
    let mut store = fixture.store();
    store
        .create_provider(
            "referenced",
            ProviderWrite::overwrite(Provider::default(), "sk-live".to_owned()),
        )
        .unwrap();
    let before = fixture.secrets().calls().len();

    store.delete_provider("legacy").unwrap();
    store.delete_provider("unset").unwrap();

    assert_eq!(
        fixture.secrets().calls().len(),
        before,
        "明文与未设置的记录没有条目可删：{:?}",
        fixture.secrets().calls()
    );

    store.delete_provider("referenced").unwrap();

    assert_eq!(
        fixture.secrets().entry(&reference("referenced")),
        None,
        "引用记录级联删掉条目"
    );
    assert!(store.get().unwrap().providers.is_empty());
}

/// 删除时条目本就不存在：按幂等成功处理，Provider 照样删掉。
#[test]
fn deleting_a_provider_whose_entry_is_gone_is_idempotent() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"{
            "version": 2,
            "providers": {
                "ollama": {
                    "api_key": "secret://io.github.xezzon.agent-maestro/provider/ollama/api_key"
                }
            }
        }"#,
    );
    let mut store = fixture.store();

    store.delete_provider("ollama").unwrap();

    assert!(
        store.get().unwrap().providers.is_empty(),
        "条目不存在也删得掉 Provider"
    );
}

/// 没有凭证的 Provider 不依赖密钥链：改端点照常保存。
///
/// 旧界面把「没配凭证」回传成空串（= 清除），若清除也要碰密钥链，密钥链一
/// 挂，连改端点都保存不了。故清除只在**记录持有引用**时才动密钥链。
#[test]
fn saving_a_provider_without_a_credential_does_not_need_the_keychain() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store
        .create_provider("ollama", ProviderWrite::unchanged(Provider::default()))
        .unwrap();
    let before = fixture.secrets().calls().len();
    fixture
        .secrets()
        .fail_next(SecretOp::Delete, unavailable(SecretOp::Delete));
    fixture
        .secrets()
        .fail_next(SecretOp::Read, unavailable(SecretOp::Read));

    store
        .update_provider(
            "ollama",
            ProviderWrite::clear(Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "http://localhost:11434/v1".to_owned(),
                )]),
                ..Provider::default()
            }),
        )
        .unwrap();

    assert_eq!(
        fixture.secrets().calls().len(),
        before,
        "没有凭证就没有条目可清：{:?}",
        fixture.secrets().calls()
    );
    assert_eq!(
        store.get().unwrap().providers["ollama"]
            .base_url
            .get(&Protocol::OpenaiCompletions)
            .map(String::as_str),
        Some("http://localhost:11434/v1"),
        "端点照常保存"
    );
}

/// 引用与所在 map key 推出的 slug 不一致：整份配置进保护态。
#[test]
fn a_reference_pointing_at_another_slug_puts_the_config_into_protected_state() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"{
            "version": 2,
            "providers": {
                "openrouter": {
                    "api_key": "secret://io.github.xezzon.agent-maestro/provider/other/api_key"
                }
            }
        }"#,
    );
    let mut store = fixture.store();

    let error = store.get().unwrap_err();
    assert!(
        matches!(error, ConfigError::SecretReferenceMismatch { .. }),
        "{error:?}"
    );
    assert!(
        store
            .create_provider("ollama", ProviderWrite::unchanged(Provider::default()))
            .is_err(),
        "保护态下拒绝一切写入"
    );
}

/// 密钥链不可用：带密钥的保存失败，配置一动不动。
#[test]
fn an_unavailable_keychain_fails_the_save_and_leaves_the_config_untouched() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    fixture
        .secrets()
        .fail_next(SecretOp::Write, unavailable(SecretOp::Write));

    let error = store
        .create_provider(
            "ollama",
            ProviderWrite::overwrite(Provider::default(), "sk-new".to_owned()),
        )
        .unwrap_err();

    assert!(matches!(error, ProviderError::Secret { .. }), "{error:?}");
    assert!(store.get().unwrap().providers.is_empty(), "配置没改动");
    assert!(
        !fixture.paths().config_path().exists(),
        "报错路径不得静默写入文件"
    );
}

/// 密钥链不可用：删除带引用的 Provider 失败，Provider 保留。
#[test]
fn an_unavailable_keychain_fails_the_delete_and_keeps_the_provider() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store
        .create_provider(
            "ollama",
            ProviderWrite::overwrite(Provider::default(), "sk-live".to_owned()),
        )
        .unwrap();
    let before = config_text(&fixture);
    fixture
        .secrets()
        .fail_next(SecretOp::Delete, unavailable(SecretOp::Delete));

    let error = store.delete_provider("ollama").unwrap_err();

    assert!(matches!(error, ProviderError::Secret { .. }), "{error:?}");
    assert!(
        store.get().unwrap().providers.contains_key("ollama"),
        "Provider 保留"
    );
    assert_eq!(config_text(&fixture), before, "配置没改动");
}

/// 落盘失败：密钥链回滚到变更前的状态（旧值写回）。
#[test]
fn a_failed_persist_restores_the_previous_keychain_value() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store
        .create_provider(
            "ollama",
            ProviderWrite::overwrite(Provider::default(), "sk-old".to_owned()),
        )
        .unwrap();
    break_persist(&fixture);

    let error = store
        .update_provider(
            "ollama",
            ProviderWrite::overwrite(Provider::default(), "sk-new".to_owned()),
        )
        .unwrap_err();

    assert!(matches!(error, ProviderError::Storage(_)), "{error:?}");
    assert_eq!(
        fixture.secrets().entry(&reference("ollama")),
        Some("sk-old".to_owned()),
        "密钥链回滚到旧值"
    );
}

/// 落盘失败：变更前没有条目就删掉新写的那个。
#[test]
fn a_failed_persist_removes_the_entry_it_had_written() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    break_persist(&fixture);

    let error = store
        .create_provider(
            "ollama",
            ProviderWrite::overwrite(Provider::default(), "sk-new".to_owned()),
        )
        .unwrap_err();

    assert!(matches!(error, ProviderError::Storage(_)), "{error:?}");
    assert_eq!(
        fixture.secrets().entry(&reference("ollama")),
        None,
        "新写的条目被删掉，不留孤儿"
    );
}

/// 回滚本身也失败：两次失败一并点名。
#[test]
fn a_failed_rollback_reports_both_failures() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    break_persist(&fixture);
    fixture
        .secrets()
        .fail_next(SecretOp::Delete, unavailable(SecretOp::Delete));

    let error = store
        .create_provider(
            "ollama",
            ProviderWrite::overwrite(Provider::default(), "sk-new".to_owned()),
        )
        .unwrap_err();

    assert!(
        matches!(error, ProviderError::SecretRollback { .. }),
        "{error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("落盘失败") && message.contains("回滚"),
        "两次失败都要点名：{message}"
    );
    assert!(
        !message.contains("sk-new"),
        "错误消息不得携带真值：{message}"
    );
}

/// v1 配置照常打开：迁移写不进密钥链时，明文原样留着，配置仍是 v1。
///
/// 「明文继续以明文工作」只在迁移失败时成立（#147）：密钥链可用时明文在
/// 启动时就换成了引用，配置随之升到 v2。
#[test]
fn a_v1_config_keeps_working_with_plaintext_when_the_keychain_is_unavailable() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"{
            "version": 1,
            "providers": { "ollama": { "api_key": "sk-legacy" } }
        }"#,
    );
    fixture
        .secrets()
        .fail_next(SecretOp::Write, unavailable(SecretOp::Write));
    let mut store = fixture.store();

    assert_eq!(
        store.get().unwrap().providers["ollama"].api_key.plaintext(),
        Some("sk-legacy"),
        "迁移失败时旧明文照常读入"
    );

    // 一次无关的落盘：版本仍是 v1（全部仍是明文），明文一字不丢。
    store.set_variables(BTreeMap::new()).unwrap();

    let text = config_text(&fixture);
    assert!(
        text.contains(r#""version": 1"#),
        "全部仍是明文的 v1 暂留 v1：{text}"
    );
    assert!(text.contains("sk-legacy"), "明文原样保留：{text}");
}

/// 出现引用即写 v2：旧版 Maestro 见到引用会当成 API Key，故必须自报 v2。
#[test]
fn a_config_with_a_reference_is_written_as_v2() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"{
            "version": 1,
            "providers": { "ollama": { "api_key": "sk-legacy" } }
        }"#,
    );
    let mut store = fixture.store();

    store
        .update_provider(
            "ollama",
            ProviderWrite::overwrite(Provider::default(), "sk-new".to_owned()),
        )
        .unwrap();

    let text = config_text(&fixture);
    assert!(text.contains(r#""version": 2"#), "出现引用即写 v2：{text}");
    assert!(!text.contains("sk-legacy"), "旧明文被引用取代：{text}");
}

/// 版本号对齐不依赖迁移：v1 文件里已经有引用（旧版回滚或手改写出）时，
/// 加载即把它改成 v2——否则旧版 Maestro 会把引用当成 API Key 投影出去。
#[test]
fn a_v1_document_that_already_holds_a_reference_is_promoted_to_v2_on_load() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"{
            "version": 1,
            "providers": {
                "ollama": {
                    "api_key": "secret://io.github.xezzon.agent-maestro/provider/ollama/api_key"
                }
            }
        }"#,
    );

    let _store = fixture.store();

    let text = config_text(&fixture);
    assert!(
        text.contains(r#""version": 2"#),
        "含引用的 v1 在加载时升为 v2：{text}"
    );
    assert!(text.contains("secret://"), "引用原样保留：{text}");
}

/// 首次写入引用即写 v2：从「无明文、无存量条目」起步——否则启动迁移会
/// 先把文件写成 v2，这条断言就失去鉴别力。
#[test]
fn a_first_reference_write_is_persisted_as_v2() {
    let fixture = Fixture::new();
    fixture.write_config(r#"{ "version": 1, "providers": {} }"#);
    let mut store = fixture.store();

    store
        .create_provider(
            "ollama",
            ProviderWrite::overwrite(Provider::default(), "sk-new".to_owned()),
        )
        .unwrap();

    let text = config_text(&fixture);
    assert!(
        text.contains(r#""version": 2"#),
        "首次写入引用即写 v2：{text}"
    );
}

/// 非法 slug 在创建时即拒绝：造不出规范引用的 slug 没有地方放真值，这条
/// 记录此后永远设不了密钥。与本次是否带凭证无关。
#[test]
fn create_provider_rejects_a_slug_that_cannot_hold_a_key() {
    let fixture = Fixture::new();
    let mut store = fixture.store();

    for slug in ["", "OpenRouter", "2fast", "open router", "open/router"] {
        let error = store
            .create_provider(slug, ProviderWrite::unchanged(Provider::default()))
            .unwrap_err();

        assert!(
            matches!(error, ProviderError::InvalidSlug { .. }),
            "slug={slug} 应被拒绝：{error:?}"
        );
    }
    assert!(
        store.providers_snapshot().unwrap().is_empty(),
        "被拒绝的 slug 一条都不该落进配置"
    );
}

/// 版本号永不回退：已是 v2 的配置不会因为引用被清掉而退回 v1。
#[test]
fn the_version_never_goes_backwards() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store
        .create_provider(
            "ollama",
            ProviderWrite::overwrite(Provider::default(), "sk-new".to_owned()),
        )
        .unwrap();

    store
        .update_provider("ollama", ProviderWrite::clear(Provider::default()))
        .unwrap();

    let text = config_text(&fixture);
    assert!(text.contains(r#""version": 2"#), "版本号不回退：{text}");
    assert!(!text.contains("secret://"), "引用清掉后不再有引用：{text}");
}

// ---------------------------------------------------------------------------
// 启动迁移（#147）：旧明文 API Key 一次性换进密钥链。
//
// 这一段钉的是**存储层接缝**：临时配置文件里放一份带明文的旧配置，打开
// 存储即触发迁移；观察到的只有「密钥链里有什么」「配置文件里有什么」
// ——真值任何时候都不该出现在配置或断言输出里。
// ---------------------------------------------------------------------------

/// 启动即迁移：旧明文搬进密钥链，配置文件只剩引用。
#[test]
fn startup_migrates_plaintext_api_keys_into_the_keychain() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"{
            "version": 1,
            "providers": { "ollama": { "api_key": "sk-legacy" } }
        }"#,
    );

    let store = fixture.store();

    assert_eq!(
        fixture.secrets().entry(&reference("ollama")),
        Some("sk-legacy".to_owned()),
        "真值搬进了密钥链"
    );
    assert_eq!(
        store.get().unwrap().providers["ollama"]
            .api_key
            .reference()
            .map(SecretRef::slug),
        Some("ollama"),
        "内存里的字段是规范引用"
    );
    let text = config_text(&fixture);
    assert!(
        text.contains("secret://io.github.xezzon.agent-maestro/provider/ollama/api_key"),
        "配置文件只剩引用：{text}"
    );
    assert!(!text.contains("sk-legacy"), "配置文件不得含真值：{text}");
    assert!(
        text.contains(r#""version": 2"#),
        "含引用的配置按 v2 落盘：{text}"
    );
}

/// 部分失败：一个 Provider 迁走了，另一个的明文原样留着，启动照常。
#[test]
fn a_failed_migration_keeps_the_plaintext_and_does_not_block_startup() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"{
            "version": 1,
            "providers": {
                "alpha": { "api_key": "sk-alpha" },
                "beta": { "api_key": "sk-beta" }
            }
        }"#,
    );
    // 迁移按 slug 序逐个尝试：第一个（alpha）写不进密钥链。
    fixture
        .secrets()
        .fail_next(SecretOp::Write, unavailable(SecretOp::Write));

    let store = fixture.store();

    assert_eq!(
        fixture.secrets().entry(&reference("beta")),
        Some("sk-beta".to_owned()),
        "成功的那个真值搬进了密钥链"
    );
    assert_eq!(
        store.get().unwrap().providers["beta"]
            .api_key
            .reference()
            .map(SecretRef::slug),
        Some("beta"),
        "成功的那个换成了引用"
    );
    assert_eq!(
        store.get().unwrap().providers["alpha"].api_key.plaintext(),
        Some("sk-alpha"),
        "失败的明文原样保留"
    );
    let text = config_text(&fixture);
    assert!(text.contains("sk-alpha"), "失败者的明文写回配置：{text}");
    assert!(!text.contains("sk-beta"), "成功者的明文不再落盘：{text}");
}

/// 迁移失败记一条 ERROR：只含 slug 与原因，绝不含密钥值。
#[test]
fn a_failed_migration_logs_the_slug_and_the_reason_without_the_secret() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"{
            "version": 1,
            "providers": { "alpha": { "api_key": "sk-canary-4b7e" } }
        }"#,
    );
    fixture
        .secrets()
        .fail_next(SecretOp::Write, unavailable(SecretOp::Write));

    let window = capture_window();
    let _store = fixture.store();
    let lines = window.lines();

    assert_eq!(lines.len(), 1, "一个失败一条日志：{lines:?}");
    assert!(lines[0].contains("[ERROR]"), "失败记 ERROR：{lines:?}");
    assert!(lines[0].contains("alpha"), "日志点名 Provider：{lines:?}");
    assert!(
        lines[0].contains("密钥链不可用"),
        "日志带失败原因：{lines:?}"
    );
    assert!(
        !lines[0].contains("sk-canary-4b7e"),
        "日志不得携带真值：{lines:?}"
    );
}

/// slug 不合法就构造不出引用：明文留着，失败原因点名 slug。
///
/// 这种 Provider 永远迁不掉（除非改 slug）——但密钥一个都不丢，下次启动
/// 再试一次，代价只是一条 ERROR。
#[test]
fn a_provider_with_an_invalid_slug_stays_plaintext() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"{
            "version": 1,
            "providers": { "Ollama": { "api_key": "sk-legacy" } }
        }"#,
    );

    let window = capture_window();
    let store = fixture.store();
    let lines = window.lines();

    assert_eq!(lines.len(), 1, "一条失败一条日志：{lines:?}");
    assert!(lines[0].contains("Ollama"), "日志点名 Provider：{lines:?}");
    assert_eq!(
        store.get().unwrap().providers["Ollama"].api_key.plaintext(),
        Some("sk-legacy"),
        "构造不出引用，明文只能留着"
    );
    assert!(
        fixture.secrets().calls().is_empty(),
        "压根没有条目可写：{:?}",
        fixture.secrets().calls()
    );
}

/// 一个都没迁成时不落盘：配置没被改动，落盘压根不尝试。
///
/// 目录只读 + 密钥链写不进去：若落盘被尝试过，它必失败并记一条 ERROR——
/// 日志里只有迁移失败那一条，即证明落盘没被尝试。
#[cfg(unix)]
#[test]
fn a_migration_that_changed_nothing_does_not_persist() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"{
            "version": 1,
            "providers": { "ollama": { "api_key": "sk-legacy" } }
        }"#,
    );
    fixture
        .secrets()
        .fail_next(SecretOp::Write, unavailable(SecretOp::Write));
    let original = freeze_persist(&fixture);

    let window = capture_window();
    let store = fixture.store();
    let lines = window.lines();

    assert_eq!(lines.len(), 1, "只有迁移失败那一条：{lines:?}");
    assert!(
        !lines[0].contains("persist"),
        "一个都没迁成就不该落盘：{lines:?}"
    );
    assert_eq!(
        store.get().unwrap().providers["ollama"].api_key.plaintext(),
        Some("sk-legacy"),
        "明文原样留着"
    );
    thaw_persist(&fixture, original);
}

/// 落盘失败：内存状态回滚为迁移前的样子，本次写进密钥链的条目留着。
///
/// 恢复可写后再开一次存储即「重启」：迁移重跑，这次落盘成功，配置只剩
/// 引用——「下次启动重试」的路径成立。
#[cfg(unix)]
#[test]
fn a_failed_migration_persist_rolls_the_memory_back_and_keeps_the_keychain_entry() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"{
            "version": 1,
            "providers": { "ollama": { "api_key": "sk-canary-4b7e" } }
        }"#,
    );
    let original = freeze_persist(&fixture);

    let window = capture_window();
    let store = fixture.store();
    let lines = window.lines();

    assert_eq!(lines.len(), 1, "落盘失败记一条日志：{lines:?}");
    assert!(lines[0].contains("[ERROR]"), "落盘失败记 ERROR：{lines:?}");
    assert!(
        lines[0].contains("slugs=ollama"),
        "日志点名迁成的 Provider：{lines:?}"
    );
    assert!(
        !lines[0].contains("sk-canary-4b7e"),
        "日志不得携带真值：{lines:?}"
    );
    assert_eq!(
        fixture.secrets().entry(&reference("ollama")),
        Some("sk-canary-4b7e".to_owned()),
        "本次写进密钥链的条目留着（惰性残余，同 slug 下次写入时覆盖）"
    );
    assert_eq!(
        store.get().unwrap().providers["ollama"].api_key.plaintext(),
        Some("sk-canary-4b7e"),
        "内存状态回滚为迁移前的样子"
    );
    let text = config_text(&fixture);
    assert!(text.contains("sk-canary-4b7e"), "配置文件没被改写：{text}");

    // 恢复可写后再开一次存储即「重启」：迁移重跑并落盘成功。
    thaw_persist(&fixture, original);
    let restarted = fixture.store();

    assert_eq!(
        restarted.get().unwrap().providers["ollama"]
            .api_key
            .reference()
            .map(SecretRef::slug),
        Some("ollama"),
        "重启后迁移重跑，字段换成了引用"
    );
    let text = config_text(&fixture);
    assert!(
        text.contains("secret://io.github.xezzon.agent-maestro/provider/ollama/api_key"),
        "配置只剩引用：{text}"
    );
    assert!(!text.contains("sk-canary-4b7e"), "配置不得含真值：{text}");
}

/// 清除明文不需要密钥链：明文没有条目可删，密钥链不可用也清得掉。
///
/// 明文只在迁移失败时残留（#147）：它压根不指向密钥链条目，故清除压根不
/// 碰密钥链。
#[test]
fn clearing_a_plaintext_credential_needs_no_keychain() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"{
            "version": 1,
            "providers": { "ollama": { "api_key": "sk-legacy" } }
        }"#,
    );
    // 迁移写不进密钥链：明文原样留在配置里。
    fixture
        .secrets()
        .fail_next(SecretOp::Write, unavailable(SecretOp::Write));
    let mut store = fixture.store();
    // 就算密钥链真的不可用，清除明文也不该受影响（压根没有条目可删）。
    fixture
        .secrets()
        .fail_next(SecretOp::Delete, unavailable(SecretOp::Delete));

    store
        .update_provider("ollama", ProviderWrite::clear(Provider::default()))
        .unwrap();

    assert!(
        !store.get().unwrap().providers["ollama"].api_key.is_set(),
        "明文被清掉了"
    );
    assert!(
        !fixture
            .secrets()
            .calls()
            .iter()
            .any(|call| call.op == SecretOp::Delete),
        "明文没有条目可删，压根不该发起删除：{:?}",
        fixture.secrets().calls()
    );
}

/// 更换密钥顺带完成迁移：明文 Provider 被覆盖后，字段是引用，真值在密钥链。
///
/// 迁移失败留下的明文不必等下次启动：用户重新设置一次，明文即换成引用
/// （ADR 0002 的写入路径「先密钥链、后配置」）。
#[test]
fn replacing_a_plaintext_credential_completes_the_migration() {
    let fixture = Fixture::new();
    fixture.write_config(
        r#"{
            "version": 1,
            "providers": { "ollama": { "api_key": "sk-legacy" } }
        }"#,
    );
    // 迁移写不进密钥链：明文原样留在配置里。
    fixture
        .secrets()
        .fail_next(SecretOp::Write, unavailable(SecretOp::Write));
    let mut store = fixture.store();

    store
        .update_provider(
            "ollama",
            ProviderWrite::overwrite(Provider::default(), "sk-new".to_owned()),
        )
        .unwrap();

    assert_eq!(
        fixture.secrets().entry(&reference("ollama")),
        Some("sk-new".to_owned()),
        "真值在密钥链里"
    );
    assert_eq!(
        store.get().unwrap().providers["ollama"]
            .api_key
            .reference()
            .map(SecretRef::slug),
        Some("ollama"),
        "字段换成了引用"
    );
    let text = config_text(&fixture);
    assert!(!text.contains("sk-legacy"), "旧明文不再落盘：{text}");
}

/// 真值任何时候都不出现在错误消息与 Debug 输出里。
#[test]
fn the_secret_never_shows_up_in_errors_or_debug_output() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    fixture
        .secrets()
        .fail_next(SecretOp::Write, unavailable(SecretOp::Write));

    let error = store
        .create_provider(
            "ollama",
            ProviderWrite::overwrite(Provider::default(), "sk-canary-4b7e".to_owned()),
        )
        .unwrap_err();

    let rendered = format!("{error:?} {error}");
    assert!(
        !rendered.contains("sk-canary-4b7e"),
        "错误不得携带真值：{rendered}"
    );
    let config = format!("{:?}", store.get().unwrap());
    assert!(
        !config.contains("sk-canary-4b7e"),
        "配置 Debug 不得携带真值：{config}"
    );
}
