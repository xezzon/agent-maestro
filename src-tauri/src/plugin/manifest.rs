//! 插件 manifest：插件 metadata 的唯一来源（插件根目录的 `manifest.json`）。
//!
//! 宿主直接读文件，组件不导出 get-metadata；容忍未知字段，必填字段缺失或
//! 插件 id 不合法即加载失败（见 issue #34）。`config_dir` 的声明语法与逃逸
//! 校验也在这里（ADR 0011）；「平台变量前缀落在哪个目录」由 [`crate::paths`]
//! 回答——本模块只管声明长什么样、能不能落到基准目录之内。

use serde::Deserialize;
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use crate::{errors::PluginError, paths::Paths};

use super::source::{PluginSource, is_https_url};

/// 插件 metadata 的唯一来源：插件根目录的 `manifest.json`。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct Manifest {
    /// 插件 id：`[a-z][a-z0-9_-]*`。
    pub(crate) id: String,
    /// 插件被授权写入的配置目录：字符串形式以平台变量前缀声明（`$HOME/.pi`、
    /// `$XDG_CONFIG_HOME/zed`…），对象形式按平台键各声明一条（键固定为
    /// `linux`、`macos`、`windows`）。反序列化只产出**声明**（形态已校验），
    /// 解析为宿主绝对路径是显式的一步（见 [`DeclaredConfigDir::resolve`]）。
    #[serde(deserialize_with = "deserialize_config_dir")]
    pub(crate) config_dir: DeclaredConfigDir,
    /// 入口 wasm 的回源地址（约束按来源种类分列，见 ADR 0006）。
    pub(crate) entry: String,
    /// 可选的内联 JSON Schema（draft 2020-12，ADR 0017）：描述该插件 `form`
    /// 段的形状，原样透传给前端渲染，宿主不校验其内容；缺省即无需用户填写。
    #[serde(default)]
    pub(crate) settings_schema: Option<serde_json::Value>,
}

/// 解析计数（测试期）：本线程累计解析了多少份 manifest。
///
/// [`parse_placed`] 是 manifest 解析的唯一入口，计数落在它身上——「一次安装
/// 只解析一次 manifest」因此是可执行断言，而不是靠读代码确认。
#[cfg(test)]
pub(crate) fn parse_count() -> usize {
    PARSE_COUNT.with(Cell::get)
}

#[cfg(test)]
thread_local! {
    /// 解析计数（测试期，每线程一份）：见 [`parse_count`]。
    static PARSE_COUNT: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
use std::cell::Cell;

/// 插件被授权写入的配置目录：**声明**，尚未解析为宿主路径。
///
/// 声明形式是「变量前缀 + 相对片段」（`$HOME/.pi`、`$XDG_CONFIG_HOME/zed`）；
/// `~/.pi` 与 `$HOME/.pi` 同义。
///
/// 本类型只经由反序列化构造，构造时已过一遍**纯语法**检查（不读目录、不碰
/// 文件系统）：变量前缀必须是受支持的平台变量，相对片段不得为空、不得是绝对
/// 路径、不得含 `..` 或裸 `.`。因此本类型保证声明的形态合法——「落到哪个
/// 目录」由 [`DeclaredConfigDir::resolve`] 显式解析（ADR 0022）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeclaredConfigDir {
    /// 声明原文（错误信息里给用户看）。
    declared: String,
    /// 平台变量前缀（`$HOME` 等；`~` 是 `$HOME` 的别名）。
    variable: String,
    /// 相对片段（不含变量前缀）。
    relative: PathBuf,
}

impl DeclaredConfigDir {
    /// 语法检查（纯判断）：把声明拆成变量前缀与相对片段，不读目录、不碰文件系统。
    ///
    /// 语法错误在解析期就报：manifest 不合法就不该触发 wasm 下载，更不该留下目录。
    fn parse(declared: &str) -> Result<Self, PluginError> {
        if declared.is_empty() {
            return Err(PluginError::ConfigDirMissing);
        }
        // 绝对路径以 `/` 开头，切出的变量前缀为空：与「没有 `/`」一样报前缀错误，
        // 不要误报成「不支持的变量」。
        let (variable, relative) = declared
            .split_once('/')
            .filter(|(variable, _)| !variable.is_empty())
            .ok_or_else(|| PluginError::ConfigDirVariableRequired {
                declared: declared.to_owned(),
                supported: &crate::paths::SUPPORTED_VARIABLES,
            })?;
        // `~` 与 `$HOME` 同义：两者解析到同一基准目录。
        if variable != "~" && !crate::paths::SUPPORTED_VARIABLES.contains(&variable) {
            return Err(PluginError::ConfigDirUnsupportedVariable {
                declared: declared.to_owned(),
                variable: variable.to_owned(),
                supported: &crate::paths::SUPPORTED_VARIABLES,
            });
        }
        let relative = PathBuf::from(relative);
        if relative.as_os_str().is_empty() {
            return Err(PluginError::ConfigDirMissingRelative {
                declared: declared.to_owned(),
            });
        }
        // 相对片段必须始终落在基准目录内部：绝对路径（`$HOME//etc`）、`..` 上跳与裸 `.
        // `在同一条路径上拒绝，不依赖后面的规范化回退。裸 `.` 单列是因为它把基准目录整盘交出去。
        if relative.components().any(|component| {
            matches!(
                component,
                Component::RootDir
                    | Component::Prefix(_)
                    | Component::ParentDir
                    | Component::CurDir
            )
        }) {
            return Err(PluginError::ConfigDirUnsafeRelative {
                declared: declared.to_owned(),
            });
        }
        Ok(Self {
            declared: declared.to_owned(),
            variable: variable.to_owned(),
            relative,
        })
    }

    /// 解析为宿主绝对路径：建目录、规范化、逃逸校验（ADR 0011）。
    ///
    /// 这一步**有副作用**（创建目录），因此是显式的一步：解析 manifest 不碰
    /// 文件系统（ADR 0022）。调用方在链接期校验通过之后才调它——wasm 不合法
    /// 时不留配置目录。
    pub(crate) fn resolve(&self, paths: &Paths) -> Result<PathBuf, PluginError> {
        let base = paths
            .base_dir(&self.variable)
            .ok_or_else(|| PluginError::ConfigDirUnsupportedVariable {
                declared: self.declared.clone(),
                variable: self.variable.clone(),
                supported: &crate::paths::SUPPORTED_VARIABLES,
            })?
            .to_path_buf();

        let dir = base.join(&self.relative);
        fs::create_dir_all(&dir).map_err(|e| PluginError::ConfigDirCreate {
            detail: e.to_string(),
        })?;
        // 目录可能是符号链接：以规范化后的真实路径确认它仍在基准目录内。
        // 必须严格子代——相等（裸 `.` 已被语法检查拒掉；但以防万一把这一道关补上）
        // 也视为逃逸，避免把整盘基准目录交出去。
        let canonical_base =
            base.canonicalize()
                .map_err(|e| PluginError::ConfigDirResolveBase {
                    detail: e.to_string(),
                })?;
        let canonical = dir
            .canonicalize()
            .map_err(|e| PluginError::ConfigDirResolve {
                detail: e.to_string(),
            })?;
        if canonical == canonical_base || !canonical.starts_with(&canonical_base) {
            return Err(PluginError::ConfigDirEscape {
                declared: self.declared.clone(),
            });
        }
        Ok(canonical)
    }
}

/// `config_dir` 字段的反序列化：声明经 [`DeclaredConfigDir::parse`] 校验形态，
/// 产出**声明**（不解析成路径、不建目录）。
///
/// 两种声明形式：字符串（单一声明，各平台共用）或对象（键固定为 `linux`、`macos`、
/// `windows`，宿主取当前平台的键，见 [`platform_declaration`]）。
fn deserialize_config_dir<'de, D>(deserializer: D) -> Result<DeclaredConfigDir, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let declared = match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::String(declared) => declared,
        serde_json::Value::Object(map) => {
            platform_declaration(&map, current_platform_key()).map_err(serde::de::Error::custom)?
        }
        _ => {
            return Err(serde::de::Error::custom(PluginError::ConfigDirShape));
        }
    };
    DeclaredConfigDir::parse(&declared).map_err(serde::de::Error::custom)
}

/// 当前平台在 config_dir 对象形式里对应的键；其余 Unix 平台沿用 `linux` 键。
fn current_platform_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

/// 从 config_dir 对象声明里取 `platform` 平台的声明。
///
/// 键固定为 `linux`、`macos`、`windows`，未知键与非字符串值一律拒绝；
/// 取出的值交给 [`DeclaredConfigDir::parse`] 走与字符串形式同一套变量语法。`platform` 由
/// 反序列化传入 [`current_platform_key`]，独立成参数以便测试覆盖三个平台的选取。
fn platform_declaration(
    map: &serde_json::Map<String, serde_json::Value>,
    platform: &str,
) -> Result<String, PluginError> {
    for (key, value) in map {
        if !matches!(key.as_str(), "linux" | "macos" | "windows") {
            return Err(PluginError::PlatformKeyUnsupported { key: key.clone() });
        }
        if !value.is_string() {
            return Err(PluginError::PlatformValueNotString { key: key.clone() });
        }
    }
    map.get(platform)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| PluginError::PlatformMissing {
            platform: platform.to_owned(),
        })
}

/// 解析并校验落位 manifest；插件 id 与 `entry` 由这里把关，`config_dir` 的**声明**
/// 在反序列化时已校验形态（不解析成路径、不建目录）。
pub(crate) fn parse_placed(value: &str) -> Result<Manifest, PluginError> {
    #[cfg(test)]
    PARSE_COUNT.with(|count| count.set(count.get() + 1));
    let manifest: Manifest =
        serde_json::from_str(value).map_err(|e| PluginError::InvalidManifest {
            detail: e.to_string(),
        })?;
    if !is_valid_plugin_id(&manifest.id) {
        return Err(PluginError::InvalidPluginId {
            id: manifest.id.clone(),
        });
    }
    if manifest.entry.is_empty() {
        return Err(PluginError::EmptyEntry);
    }
    Ok(manifest)
}

/// 解析并校验 manifest 文本；`entry` 的约束按来源分列。
pub(crate) fn parse_manifest(source: &PluginSource, text: &str) -> Result<Manifest, PluginError> {
    let manifest = parse_placed(text)?;
    match source {
        // wasm 内嵌于二进制，entry 不参与解析。
        PluginSource::Builtin(_) => {}
        PluginSource::Https(_) => {
            if !is_https_url(&manifest.entry) {
                return Err(PluginError::HttpsEntryRequired {
                    entry: manifest.entry.clone(),
                });
            }
        }
        // file 来源的 entry 是插件目录内的相对路径或 https URL（回源到网络）：绝对路径、
        // `..` 上跳与其他 scheme（http://、file://…）一律拒绝，防止外置 manifest 借 entry
        // 读取插件根目录之外的文件。
        PluginSource::File(_) => {
            if !is_https_url(&manifest.entry) {
                let entry_path = Path::new(&manifest.entry);
                if entry_path.is_absolute()
                    || entry_path.components().any(|c| c == Component::ParentDir)
                    // 带 scheme 的形态到不了联网获取分支，会在来源目录里读一个不可能
                    // 存在的文件：在校验期拒掉，而不是报误导性的读取错误。
                    || url::Url::parse(&manifest.entry).is_ok()
                {
                    return Err(PluginError::RelativeOrHttpsEntryRequired {
                        entry: manifest.entry.clone(),
                    });
                }
            }
        }
    }
    Ok(manifest)
}

/// 插件 id 规则与 Provider slug 一致（见 docs/GLOSSARY.md）。
fn is_valid_plugin_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

#[cfg(test)]
pub(crate) mod testutil {
    /// 上游 manifest 模板：三个测试模块共用同一份 JSON 形状。
    pub(crate) fn manifest_json(id: &str, config_dir: &str, entry: &str) -> String {
        format!(
            r#"{{
                    "id": "{id}",
                    "config_dir": "{config_dir}",
                    "entry": "{entry}"
                }}"#
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_text(id: &str, entry: &str) -> String {
        crate::plugin::testutil::manifest_json(id, "$HOME/.x", entry)
    }

    /// file 来源（测试里最常用的一种）。
    fn file_source() -> PluginSource {
        PluginSource::File(PathBuf::from("/tmp/plugin/manifest.json"))
    }

    /// https 来源。
    fn https_source() -> PluginSource {
        PluginSource::Https("https://example.com/manifest.json".to_owned())
    }

    /// 内置来源。
    fn builtin_source() -> PluginSource {
        PluginSource::Builtin("pi".to_owned())
    }

    /// 基准目录：全部落在临时目录下，不读真实环境。
    ///
    /// 语法检查（[`DeclaredConfigDir::parse`]）与路径解析（`resolve`）都接收
    /// [`Paths`] 值、不读进程级全局——夹具因此不注入全局，直接返回值（#141 I-4）。
    fn platform_dirs_setup() -> (tempfile::TempDir, Paths) {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::distinct_under(root.path());
        (root, paths)
    }

    #[test]
    fn manifest_parses_required_fields_and_tolerates_unknown_fields() {
        let (_root, _paths) = platform_dirs_setup();
        let manifest = parse_manifest(
            &file_source(),
            r#"{
                "id": "pi",
                "name": "Pi",
                "tool": "pi",
                "config_dir": "$HOME/.pi",
                "entry": "plugin.wasm",
                "author": "someone",
                "version": "0.2.0"
            }"#,
        )
        .unwrap();

        assert_eq!(manifest.id, "pi");
        assert_eq!(manifest.config_dir.declared, "$HOME/.pi");
        assert_eq!(manifest.entry, "plugin.wasm");
    }

    /// 解析不碰文件系统：解析出的只是**声明**，声明指向的目录此刻并不存在。
    ///
    /// 这是「解析即建目录」被拆掉后的核心断言（ADR 0022）：解析一个来自网络
    /// 的 manifest 不得在用户主目录下留下任何目录。
    #[test]
    fn parsing_a_manifest_creates_no_directory() {
        let (root, _paths) = platform_dirs_setup();

        let manifest = parse_manifest(&file_source(), &manifest_text("pi", "p.wasm")).unwrap();

        assert_eq!(manifest.config_dir.declared, "$HOME/.x");
        assert!(
            !root.path().join("home").join(".x").exists(),
            "解析不得建出声明指向的目录：{}",
            root.path().join("home").join(".x").display()
        );
    }

    /// `settings_schema`（ADR 0017）可选：存在即整段透传，宿主不校验其内容；
    /// 缺省即 None（该插件无需用户填写表单）。
    #[test]
    fn manifest_settings_schema_is_passed_through_verbatim_and_optional() {
        let (_root, _paths) = platform_dirs_setup();
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "model": { "type": "string", "title": "模型" } },
            "required": ["model"]
        });
        let text = format!(
            r#"{{
                    "id": "zed",
                    "config_dir": "$HOME/.zed",
                    "entry": "p.wasm",
                    "settings_schema": {schema}
                }}"#
        );

        let manifest = parse_manifest(&file_source(), &text).unwrap();
        assert_eq!(manifest.settings_schema, Some(schema));

        let without_schema = manifest_text("zed", "p.wasm");
        assert_eq!(
            parse_manifest(&file_source(), &without_schema)
                .unwrap()
                .settings_schema,
            None
        );
    }

    /// `settings_schema` 的字段顺序照抄 manifest（ADR 0017）：「原样透传」到前端后，
    /// rjsf 按 `properties` 的顺序排布表单字段，序列化时不得按字典序重排。
    #[test]
    fn settings_schema_property_order_matches_the_manifest() {
        let (_root, _paths) = platform_dirs_setup();
        let text = r#"{
                "id": "zed",
                "config_dir": "$HOME/.zed",
                "entry": "p.wasm",
                "settings_schema": {
                    "type": "object",
                    "properties": {
                        "zz_later": { "type": "string" },
                        "aa_earlier": { "type": "string" }
                    }
                }
            }"#;

        let manifest = parse_manifest(&file_source(), text).unwrap();

        let serialized = serde_json::to_string(&manifest.settings_schema).unwrap();
        assert!(
            serialized.find("zz_later") < serialized.find("aa_earlier"),
            "字段顺序应与 manifest 一致：{serialized}"
        );
    }

    #[test]
    fn manifest_missing_required_field_is_rejected() {
        // 只留 id：其余必填缺失即加载失败。
        let err = parse_manifest(&file_source(), r#"{"id": "pi"}"#).unwrap_err();
        assert!(err.contains("manifest.json 不合法"), "{err}");
    }

    #[test]
    fn manifest_with_invalid_id_is_rejected() {
        let (_root, _paths) = platform_dirs_setup();
        for id in ["Pi", "1pi", "-pi", "pi.", "pi 中文", ""] {
            let text = manifest_text(id, "p.wasm");
            let err = parse_manifest(&file_source(), &text).unwrap_err();
            assert!(err.contains("id"), "id {id:?} 应被拒绝：{err}");
        }
    }

    #[test]
    fn valid_plugin_ids_are_accepted() {
        assert!(is_valid_plugin_id("pi"));
        assert!(is_valid_plugin_id("a1"));
        assert!(is_valid_plugin_id("some-plugin_2"));
        assert!(!is_valid_plugin_id(""));
        assert!(!is_valid_plugin_id("Pi"));
        assert!(!is_valid_plugin_id("1pi"));
        assert!(!is_valid_plugin_id("_pi"));
        assert!(!is_valid_plugin_id("pi."));
        assert!(!is_valid_plugin_id("pi x"));
    }

    /// 解析（纯）与解析成路径（显式一步）分开：语法检查在解析期，落地在
    /// resolve 期——每个平台变量各自落到自己的基准目录之下，目录不存在时
    /// 由 resolve 建出。
    #[test]
    fn config_dir_resolves_each_platform_variable_into_its_base_dir() {
        let (root, paths) = platform_dirs_setup();

        for (declared, base, relative) in [
            ("$HOME/.pi", "home", ".pi"),
            ("$XDG_CONFIG_HOME/zed", "config", "zed"),
            ("$LOCAL_APP_CONFIG/zed", "config-local", "zed"),
            ("$XDG_DATA_HOME/app/models", "data", "app/models"),
            ("$LOCAL_APP_DATA/app", "data-local", "app"),
        ] {
            let resolved = DeclaredConfigDir::parse(declared)
                .unwrap()
                .resolve(&paths)
                .unwrap();
            assert_eq!(
                resolved,
                root.path()
                    .join(base)
                    .canonicalize()
                    .unwrap()
                    .join(relative),
                "{declared} 应解析到基准目录 {base} 之下"
            );
            assert!(
                resolved.is_dir(),
                "{declared} 指向的目录不存在时应先创建：{}",
                resolved.display()
            );
        }
    }

    #[test]
    fn config_dir_treats_tilde_as_home() {
        let (_root, paths) = platform_dirs_setup();

        let tilde = DeclaredConfigDir::parse("~/.x")
            .unwrap()
            .resolve(&paths)
            .unwrap();
        let home = DeclaredConfigDir::parse("$HOME/.x")
            .unwrap()
            .resolve(&paths)
            .unwrap();
        assert_eq!(tilde, home);
    }

    /// 对象形式：宿主按当前平台取键，取出的值仍走同一套变量语法。
    /// 三个键都给上、各指不同基准目录，断言随运行平台分派。
    #[test]
    fn config_dir_object_picks_the_current_platform_declaration() {
        let (root, paths) = platform_dirs_setup();
        let text = r#"{
            "id": "zed",
            "config_dir": {
                "linux": "$XDG_CONFIG_HOME/zed",
                "macos": "$HOME/.config/zed",
                "windows": "$LOCAL_APP_CONFIG/Zed"
            },
            "entry": "plugin.wasm"
        }"#;

        let manifest = parse_manifest(&file_source(), text).unwrap();

        let canonical_root = root.path().canonicalize().unwrap();
        let (expected_declared, expected) = match std::env::consts::OS {
            "macos" => ("$HOME/.config/zed", canonical_root.join("home/.config/zed")),
            "windows" => (
                "$LOCAL_APP_CONFIG/Zed",
                canonical_root.join("config-local/Zed"),
            ),
            _ => ("$XDG_CONFIG_HOME/zed", canonical_root.join("config/zed")),
        };
        assert_eq!(
            manifest.config_dir.declared, expected_declared,
            "对象形式按当前平台取键"
        );
        assert_eq!(
            manifest.config_dir.resolve(&paths).unwrap(),
            expected,
            "取出的声明按同一套变量语法解析"
        );
    }

    /// 平台选取独立成 [`platform_declaration`] 的参数：三个平台的键都能在任意
    /// 平台上测试，不必真跑在对应操作系统上。
    #[test]
    fn platform_declaration_selects_the_requested_platform() {
        let map = serde_json::json!({
            "linux": "$XDG_CONFIG_HOME/zed",
            "macos": "$HOME/.config/zed"
        })
        .as_object()
        .unwrap()
        .clone();

        assert_eq!(
            platform_declaration(&map, "linux").unwrap(),
            "$XDG_CONFIG_HOME/zed"
        );
        assert_eq!(
            platform_declaration(&map, "macos").unwrap(),
            "$HOME/.config/zed"
        );
        // 对象里没有的键：缺声明即失败，不静默回退到其他平台。
        let err = platform_declaration(&map, "windows").unwrap_err();
        assert!(err.contains("缺少平台"), "{err}");
        // 空对象同样缺声明。
        let err = platform_declaration(&serde_json::Map::new(), "linux").unwrap_err();
        assert!(err.contains("缺少平台"), "{err}");
    }

    #[test]
    fn config_dir_object_rejects_unknown_keys_non_string_values_and_non_object_shapes() {
        let (root, _paths) = platform_dirs_setup();

        for (config_dir, expected) in [
            // 键固定为三个平台名，未知键拒绝。
            (r#"{"freebsd": "$HOME/.x"}"#, "不支持的平台键"),
            (
                r#"{"linux": "$HOME/.x", "Linux": "$HOME/.y"}"#,
                "不支持的平台键",
            ),
            // 值必须是字符串：拿到的声明要交给声明语法检查走变量语法。
            (r#"{"linux": 1}"#, "必须是字符串"),
            // 既不是字符串也不是对象。
            ("1", "必须是字符串或按平台键声明的对象"),
        ] {
            let text =
                format!(r#"{{ "id": "zed", "config_dir": {config_dir}, "entry": "p.wasm" }}"#);
            let err = parse_manifest(&file_source(), &text).unwrap_err();
            assert!(
                err.contains(expected),
                "config_dir {config_dir} 应被拒绝：{err}"
            );
        }

        // 当前平台缺声明即失败，其余平台的键不能充当回退：排除当前平台键构造对象。
        let others: serde_json::Map<String, serde_json::Value> = ["linux", "macos", "windows"]
            .into_iter()
            .filter(|key| *key != current_platform_key())
            .map(|key| (key.to_owned(), serde_json::json!("$HOME/.x")))
            .collect();
        let err = platform_declaration(&others, current_platform_key()).unwrap_err();
        assert!(err.contains("缺少平台"), "{err}");
        assert!(
            !root.path().join("home").exists() && !root.path().join("config").exists(),
            "被拒绝的声明不得产生任何目录"
        );
    }

    /// 语法不合法的声明在**解析期**就被拒（纯判断，不读目录、不碰文件系统）：
    /// manifest 不合法就不该触发 wasm 下载，更不该留下目录。
    #[test]
    fn config_dir_rejects_shapes_outside_the_grammar() {
        let (root, _paths) = platform_dirs_setup();

        for (declared, expected) in [
            ("$FOO/.x", "不支持的平台变量"),
            ("$HOME", "必须以平台变量开头"),
            ("~", "必须以平台变量开头"),
            // 绝对路径没有变量前缀：报前缀错误，而不是「不支持的变量」。
            ("/etc/x", "必须以平台变量开头"),
            ("$HOME/", "缺少相对片段"),
            ("$HOME//etc/passwd", "不得是绝对路径"),
            ("$HOME/.x/../../etc", "不得包含「..」"),
            ("$XDG_CONFIG_HOME/../x", "不得包含「..」"),
            // 裸「.」与「./x」：和 `..` 一起列入拒绝形态。裸 `.` 单独放行会把整盘
            // 基准目录交出去（`canonical == canonical_base` 也得拒）。
            ("$HOME/.", "或「.」"),
            ("$HOME/./x", "或「.」"),
        ] {
            let err = DeclaredConfigDir::parse(declared).unwrap_err();
            assert!(err.contains(expected), "{declared} 应被拒绝：{err}");
        }
        assert!(
            !root.path().join("home").exists(),
            "被拒绝的声明不得产生任何目录"
        );
    }

    /// 报错里列出的受支持变量前缀逐字钉住：五个、跨平台固定（ADR 0011）。
    ///
    /// 该清单由调用方填进错误类型（错误类型不反向依赖 `paths`），故在此钉住，
    /// 免得清单被换掉却无人察觉——用户正是照着这句去改 `config_dir` 的。
    #[test]
    fn the_config_dir_error_lists_the_supported_variable_prefixes_verbatim() {
        let unsupported = DeclaredConfigDir::parse("$FOO/.x").unwrap_err();
        assert!(
            unsupported.contains(
                "config_dir「$FOO/.x」使用了不支持的平台变量「$FOO」（支持：$HOME、$XDG_CONFIG_HOME、$LOCAL_APP_CONFIG、$XDG_DATA_HOME、$LOCAL_APP_DATA）"
            ),
            "{unsupported}"
        );

        let no_prefix = DeclaredConfigDir::parse("$HOME").unwrap_err();
        assert!(
            no_prefix.contains(
                "config_dir「$HOME」必须以平台变量开头（支持：$HOME、$XDG_CONFIG_HOME、$LOCAL_APP_CONFIG、$XDG_DATA_HOME、$LOCAL_APP_DATA）"
            ),
            "{no_prefix}"
        );
    }

    /// 逃逸校验看的是规范化后的真实路径：基准目录内的符号链接指向外面同样被拒。
    #[cfg(unix)]
    #[test]
    fn config_dir_rejects_a_symlink_escaping_its_base_dir() {
        let (root, paths) = platform_dirs_setup();
        let outside = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        fs::create_dir_all(&home).unwrap();
        std::os::unix::fs::symlink(outside.path(), home.join("link")).unwrap();

        let err = DeclaredConfigDir::parse("$HOME/link")
            .unwrap()
            .resolve(&paths)
            .unwrap_err();

        assert!(err.contains("逃逸了基准目录"), "{err}");
    }

    /// 严格子代判定：基准目录内放一个指向基准目录自身的 symlink 即便路径段形态合法
    /// （不是 `.`、`..`、绝对路径），规范化后仍会落到基准目录本身——必须判逃逸。
    #[cfg(unix)]
    #[test]
    fn config_dir_rejects_a_path_that_canonicalizes_to_the_base_dir_itself() {
        let (root, paths) = platform_dirs_setup();
        let home = root.path().join("home");
        fs::create_dir_all(&home).unwrap();
        // `home/loop` 是指向 `home` 的 symlink。`$HOME/loop` 形态合法但规范化后
        // 就是基准目录本身——形态校验放行，逃逸校验必须兜住。
        std::os::unix::fs::symlink(&home, home.join("loop")).unwrap();

        let err = DeclaredConfigDir::parse("$HOME/loop")
            .unwrap()
            .resolve(&paths)
            .unwrap_err();

        assert!(err.contains("逃逸了基准目录"), "{err}");
    }

    #[test]
    fn file_entry_must_be_a_relative_path_without_parent_dir() {
        let (_root, _paths) = platform_dirs_setup();
        for entry in ["/etc/passwd", "../outside.wasm", "sub/../../outside.wasm"] {
            let err = parse_manifest(&file_source(), &manifest_text("pi", entry)).unwrap_err();
            assert!(err.contains("entry"), "entry {entry:?} 应被拒绝：{err}");
        }
        assert!(parse_manifest(&file_source(), &manifest_text("pi", "sub/p.wasm")).is_ok());
    }

    #[test]
    fn file_entry_rejects_non_https_schemes() {
        let (_root, _paths) = platform_dirs_setup();
        // 带 scheme 的 entry 只接受 https；http:// 等既不是相对路径也不会联网获取，
        // 必须在解析期拒绝，而不是当成相对路径去读一个不可能存在的文件。
        for entry in [
            "http://example.com/plugin.wasm",
            "file:///tmp/plugin.wasm",
            "ftp://example.com/plugin.wasm",
        ] {
            let err = parse_manifest(&file_source(), &manifest_text("pi", entry)).unwrap_err();
            assert!(
                err.contains("entry") && err.contains("https URL"),
                "entry {entry:?} 应被拒绝：{err}"
            );
        }
        assert!(
            parse_manifest(
                &file_source(),
                &manifest_text("pi", "https://example.com/plugin.wasm")
            )
            .is_ok(),
            "file 来源的 https entry 合法（回源到网络）"
        );
    }

    #[test]
    fn https_entry_must_be_an_https_url() {
        let (_root, _paths) = platform_dirs_setup();
        let ok = manifest_text("pi", "https://example.com/releases/download/v1/plugin.wasm");
        assert_eq!(
            parse_manifest(&https_source(), &ok).unwrap().entry,
            "https://example.com/releases/download/v1/plugin.wasm"
        );

        for entry in [
            "plugin.wasm",
            "sub/p.wasm",
            "/etc/passwd",
            "../outside.wasm",
            "http://example.com/plugin.wasm",
            "https://",
        ] {
            let err = parse_manifest(&https_source(), &manifest_text("pi", entry)).unwrap_err();
            assert!(
                err.contains("https URL"),
                "https 来源的 entry {entry:?} 应被拒绝：{err}"
            );
        }
    }

    #[test]
    fn builtin_entry_is_not_constrained() {
        let (_root, _paths) = platform_dirs_setup();
        // 内置插件的 wasm 内嵌于二进制，entry 不参与解析。
        let text = manifest_text("pi", "target/wasm32-wasip2/release/maestro_plugin_pi.wasm");
        assert!(parse_manifest(&builtin_source(), &text).is_ok());
    }
}
