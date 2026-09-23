use crate::plugin::builtin;
use serde::Deserialize;
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

/// 插件来源种类：决定 `entry` 的校验规则（见 ADR 0006）。
///
/// 三种来源共用同一份 manifest 校验，`entry` 语义按来源分列，
/// 不做跨协议的统一解析。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// 内置于应用：wasm 内嵌，`entry` 不参与解析。
    Builtin,
    /// https 来源：指向正式发布的 manifest，`entry` 必须是 https URL。
    Https,
    /// 本机指向 manifest.json 的绝对路径（本地调试回路）：`entry` 是 manifest
    /// 所在目录内的相对路径或 https URL。
    File,
}

impl SourceKind {
    /// 来源字符串的种类：装载与生命周期操作按此分派。
    ///
    /// `builtin:<id>` 内置于应用；https URL 为正式发布来源；本机绝对路径为 file 来源。
    /// 其余形态（如旧配置残留的 Git 地址、相对路径）无法识别，返回 `None` 由调用方
    /// 报「暂不支持的插件来源」。
    pub fn from_source(source: &str) -> Option<Self> {
        if source.starts_with(builtin::SOURCE_PREFIX) {
            Some(SourceKind::Builtin)
        } else if is_https_url(source) {
            Some(SourceKind::Https)
        } else if Path::new(source).is_absolute() {
            Some(SourceKind::File)
        } else {
            None
        }
    }
}

/// 插件 metadata 的唯一来源：插件根目录的 `manifest.json`。
///
/// 宿主直接读文件，组件不导出 get-metadata；容忍未知字段，
/// 必填字段缺失或插件 id 不合法即加载失败（见 issue #34）。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    /// 插件 id：`[a-z][a-z0-9-_]*`。
    pub id: String,
    /// 插件被授权写入的配置目录，以平台变量前缀声明（`$HOME/.pi`、
    /// `$XDG_CONFIG_HOME/zed`…）；反序列化时即由 [`resolve_config_dir`] 解析为
    /// 宿主绝对路径，构造出的 `Manifest` 只持有绝对路径。
    #[serde(deserialize_with = "deserialize_config_dir")]
    pub config_dir: PathBuf,
    /// 入口 wasm 的回源地址（约束按来源种类分列，见 ADR 0006）。
    pub entry: String,
}

#[cfg(not(test))]
static PLATFORM_DIRS: std::sync::OnceLock<PlatformDirs> = std::sync::OnceLock::new();

#[cfg(test)]
thread_local! {
    static PLATFORM_DIRS_TEST: std::cell::RefCell<Option<PlatformDirs>> =
        const { std::cell::RefCell::new(None) };
}

/// 平台基准目录：`config_dir` 声明里的变量前缀在此解析为宿主绝对路径。
#[derive(Clone, Debug)]
pub struct PlatformDirs {
    home: PathBuf,
    config: PathBuf,
    config_local: PathBuf,
    data: PathBuf,
    data_local: PathBuf,
}

impl PlatformDirs {
    /// 读取宿主真实的平台目录并初始化全局实例；任一项不可用即报错。
    /// 重复调用无效（以首次为准）。
    ///
    /// 测试构建下不读取真实环境——全局实例由 `PlatformDirs::init_test` 或
    /// `PlatformDirs::test_guard` 注入（见 `plugin::testutil`），故这里是空操作：
    /// 测试若漏了注入，由 [`PlatformDirs::get`] 报错，而不是静默落到真实目录。
    pub fn init_from_system() -> Result<(), String> {
        #[cfg(not(test))]
        {
            let missing = |what: &str| format!("无法解析平台目录：{what}不可用");
            let dirs = Self::new(
                dirs::home_dir().ok_or_else(|| missing("主目录"))?,
                dirs::config_dir().ok_or_else(|| missing("配置目录"))?,
                dirs::config_local_dir().ok_or_else(|| missing("本地配置目录"))?,
                dirs::data_dir().ok_or_else(|| missing("数据目录"))?,
                dirs::data_local_dir().ok_or_else(|| missing("本地数据目录"))?,
            );
            let _ = PLATFORM_DIRS.set(dirs);
        }
        Ok(())
    }

    /// 注入基准目录：测试用临时目录，不读真实环境。
    pub fn new(
        home: PathBuf,
        config: PathBuf,
        config_local: PathBuf,
        data: PathBuf,
        data_local: PathBuf,
    ) -> Self {
        Self {
            home,
            config,
            config_local,
            data,
            data_local,
        }
    }

    /// 获取全局实例（值语义，clone 成本极低）。
    pub fn get() -> Self {
        #[cfg(not(test))]
        {
            PLATFORM_DIRS
                .get()
                .expect("PlatformDirs 尚未初始化：调用 PlatformDirs::init_from_system 后再使用")
                .clone()
        }
        #[cfg(test)]
        {
            PLATFORM_DIRS_TEST.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .expect("测试线程未设置 PlatformDirs：使用 PlatformDirs::test_guard 或 PlatformDirs::init_test")
                    .clone()
            })
        }
    }

    /// 测试辅助：设置当前线程的全局实例，返回 RAII guard（drop 时自动清理）。
    #[cfg(test)]
    pub fn test_guard(dirs: Self) -> PlatformDirsTestGuard {
        Self::init_test(dirs);
        PlatformDirsTestGuard
    }

    /// 测试辅助：直接设置当前线程的全局实例（不返回 guard，自行管理生命周期）。
    #[cfg(test)]
    pub fn init_test(dirs: Self) {
        PLATFORM_DIRS_TEST.with(|cell| {
            *cell.borrow_mut() = Some(dirs);
        });
    }

    /// 变量前缀 → 基准目录；不支持的变量返回 `None`。
    fn base_dir(&self, variable: &str) -> Option<&Path> {
        match variable {
            "$HOME" => Some(&self.home),
            "$XDG_CONFIG_HOME" => Some(&self.config),
            "$LOCAL_APP_CONFIG" => Some(&self.config_local),
            "$XDG_DATA_HOME" => Some(&self.data),
            "$LOCAL_APP_DATA" => Some(&self.data_local),
            _ => None,
        }
    }
}

/// 测试 RAII guard：drop 时清理当前线程的 [`PlatformDirs`] 全局实例。
#[cfg(test)]
#[must_use = "guard 被立即 drop 将立即清理全局实例"]
pub struct PlatformDirsTestGuard;

#[cfg(test)]
impl Drop for PlatformDirsTestGuard {
    fn drop(&mut self) {
        PLATFORM_DIRS_TEST.with(|cell| {
            *cell.borrow_mut() = None;
        });
    }
}

/// 声明里支持的平台变量前缀（错误信息里列给用户）。
const SUPPORTED_VARIABLES: &str =
    "$HOME、$XDG_CONFIG_HOME、$LOCAL_APP_CONFIG、$XDG_DATA_HOME、$LOCAL_APP_DATA";

/// 把 manifest 声明的 `config_dir` 解析为宿主绝对路径。
///
/// 声明形式是「变量前缀 + 相对片段」（`$HOME/.pi`、`$XDG_CONFIG_HOME/zed`）；
/// `~/.pi` 与 `$HOME/.pi` 同义。目录不存在时先创建（沿用既有行为），
/// 随后按规范化的真实路径确认没有逃出该变量对应的基准目录。
///
/// 使用全局 [`PlatformDirs`] 解析，调用前需先初始化。返回的原因不带层级前缀，
/// 由 [`parse_placed`] 统一补上「manifest.json 不合法：」。
pub fn resolve_config_dir(declared: &str) -> Result<PathBuf, String> {
    if declared.is_empty() {
        return Err("config_dir 未配置".to_owned());
    }
    let dirs = PlatformDirs::get();
    // 绝对路径以 `/` 开头，切出的变量前缀为空：与「没有 `/`」一样报前缀错误，
    // 不要误报成「不支持的变量」。
    let (variable, relative) = declared
        .split_once('/')
        .filter(|(variable, _)| !variable.is_empty())
        .ok_or_else(|| {
            format!("config_dir「{declared}」必须以平台变量开头（支持：{SUPPORTED_VARIABLES}）")
        })?;
    // `~` 与 `$HOME` 同义：两者解析到同一基准目录。
    let base = if variable == "~" {
        dirs.home.clone()
    } else {
        dirs.base_dir(variable).ok_or_else(|| {
            format!(
                "config_dir「{declared}」使用了不支持的平台变量「{variable}」（支持：{SUPPORTED_VARIABLES}）"
            )
        })?.to_path_buf()
    };

    let relative = Path::new(relative);
    if relative.as_os_str().is_empty() {
        return Err(format!("config_dir「{declared}」缺少相对片段"));
    }
    // 相对片段必须始终落在基准目录内部：绝对路径（`$HOME//etc`）、`..` 上跳与裸 `.
    // `在同一条路径上拒绝，不依赖后面的规范化回退。裸 `.` 单列是因为它把基准目录整盘交出去。
    if relative.components().any(|component| {
        matches!(
            component,
            Component::RootDir | Component::Prefix(_) | Component::ParentDir | Component::CurDir
        )
    }) {
        return Err(format!(
            "config_dir「{declared}」的相对片段不得是绝对路径，也不得包含「..」或「.」"
        ));
    }

    let dir = base.join(relative);
    fs::create_dir_all(&dir).map_err(|e| format!("创建插件配置目录失败：{e}"))?;
    // 目录可能是符号链接：以规范化后的真实路径确认它仍在基准目录内。
    // 必须严格子代——相等（裸 `.` 经过上面那个检查已被拒；但以防万一把这一道关补上）
    // 也视为逃逸，避免把整盘基准目录交出去。
    let canonical_base = base
        .canonicalize()
        .map_err(|e| format!("解析平台基准目录失败：{e}"))?;
    let canonical = dir
        .canonicalize()
        .map_err(|e| format!("解析插件配置目录失败：{e}"))?;
    if canonical == canonical_base || !canonical.starts_with(&canonical_base) {
        return Err(format!("config_dir「{declared}」逃逸了基准目录"));
    }
    Ok(canonical)
}

/// `config_dir` 字段的反序列化：声明经 [`resolve_config_dir`] 立即解析为宿主绝对路径。
///
/// 解析收在反序列化里，`Manifest` 一经构造 `config_dir` 就是可用的绝对路径，不存在
/// 「已解析 / 未解析」两种状态。使用全局 [`PlatformDirs`]，反序列化前需先初始化。
fn deserialize_config_dir<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let declared = String::deserialize(deserializer)?;
    resolve_config_dir(&declared).map_err(serde::de::Error::custom)
}

/// 解析并校验落位 manifest；插件 id 与 `entry` 由这里把关，`config_dir` 已在
/// 反序列化时由 [`resolve_config_dir`] 解析为宿主绝对路径。
///
/// 使用全局 [`PlatformDirs`]，调用前需先初始化。
pub fn parse_placed(value: &str) -> Result<Manifest, String> {
    let manifest: Manifest =
        serde_json::from_str(value).map_err(|e| format!("manifest.json 不合法：{e}"))?;
    if !is_valid_plugin_id(&manifest.id) {
        return Err(format!(
            "manifest.json 不合法：插件 id「{}」需以小写字母开头，仅允许小写字母、数字、连字符和下划线",
            manifest.id
        ));
    }
    if manifest.entry.is_empty() {
        return Err("manifest.json 不合法：entry 不能为空".to_owned());
    }
    Ok(manifest)
}

/// 解析并校验 manifest 文本；`entry` 的约束按来源种类分列。
pub fn parse_manifest(kind: SourceKind, text: &str) -> Result<Manifest, String> {
    let manifest = parse_placed(text)?;
    match kind {
        // wasm 内嵌于二进制，entry 不参与解析。
        SourceKind::Builtin => {}
        SourceKind::Https => {
            if !is_https_url(&manifest.entry) {
                return Err(format!(
                    "manifest.json 不合法：https 来源的 entry「{}」必须是 https URL",
                    manifest.entry
                ));
            }
        }
        // file 来源的 entry 是插件目录内的相对路径或 https URL（回源到网络）：绝对路径、
        // `..` 上跳与其他 scheme（http://、file://…）一律拒绝，防止外置 manifest 借 entry
        // 读取插件根目录之外的文件。
        SourceKind::File => {
            if !is_https_url(&manifest.entry) {
                let entry_path = Path::new(&manifest.entry);
                if entry_path.is_absolute()
                    || entry_path.components().any(|c| c == Component::ParentDir)
                    // 带 scheme 的形态到不了联网获取分支，会在来源目录里读一个不可能
                    // 存在的文件：在校验期拒掉，而不是报误导性的读取错误。
                    || url::Url::parse(&manifest.entry).is_ok()
                {
                    return Err(format!(
                        "manifest.json 不合法：entry「{}」必须是插件目录内的相对路径或 https URL",
                        manifest.entry
                    ));
                }
            }
        }
    }
    Ok(manifest)
}

/// 是否为绝对 https URL：明文 http、其他 scheme 与空主机一律拒绝。
pub fn is_https_url(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|parsed| parsed.scheme() == "https" && parsed.has_host())
}

/// 插件 id 规则与 Provider slug 一致（见 CONTEXT.md）。
pub fn is_valid_plugin_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

#[cfg(test)]
pub mod testutil {
    /// 上游 manifest 模板：三个测试模块共用同一份 JSON 形状。
    pub fn manifest_json(id: &str, config_dir: &str, entry: &str) -> String {
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

    /// 注入的基准目录：全部落在临时目录下，不读真实环境。
    fn platform_dirs_setup() -> (tempfile::TempDir, PlatformDirsTestGuard) {
        let root = tempfile::tempdir().unwrap();
        let dirs = PlatformDirs::new(
            root.path().join("home"),
            root.path().join("config"),
            root.path().join("config-local"),
            root.path().join("data"),
            root.path().join("data-local"),
        );
        let guard = PlatformDirs::test_guard(dirs);
        (root, guard)
    }

    #[test]
    fn manifest_parses_required_fields_and_tolerates_unknown_fields() {
        let (root, _g) = platform_dirs_setup();
        let manifest = parse_manifest(
            SourceKind::File,
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
        assert_eq!(
            manifest.config_dir,
            root.path().join("home").canonicalize().unwrap().join(".pi")
        );
        assert_eq!(manifest.entry, "plugin.wasm");
    }

    #[test]
    fn manifest_missing_required_field_is_rejected() {
        // 只留 id：其余必填缺失即加载失败。
        let err = parse_manifest(SourceKind::File, r#"{"id": "pi"}"#).unwrap_err();
        assert!(err.contains("manifest.json 不合法"), "{err}");
    }

    #[test]
    fn manifest_with_invalid_id_is_rejected() {
        let (_root, _g) = platform_dirs_setup();
        for id in ["Pi", "1pi", "-pi", "pi.", "pi 中文", ""] {
            let text = manifest_text(id, "p.wasm");
            let err = parse_manifest(SourceKind::File, &text).unwrap_err();
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

    #[test]
    fn config_dir_resolves_each_platform_variable_into_its_base_dir() {
        let (root, _g) = platform_dirs_setup();

        for (declared, base, relative) in [
            ("$HOME/.pi", "home", ".pi"),
            ("$XDG_CONFIG_HOME/zed", "config", "zed"),
            ("$LOCAL_APP_CONFIG/zed", "config-local", "zed"),
            ("$XDG_DATA_HOME/app/models", "data", "app/models"),
            ("$LOCAL_APP_DATA/app", "data-local", "app"),
        ] {
            let resolved = resolve_config_dir(declared).unwrap();
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
        let (_root, _g) = platform_dirs_setup();

        assert_eq!(
            resolve_config_dir("~/.x").unwrap(),
            resolve_config_dir("$HOME/.x").unwrap()
        );
    }

    #[test]
    fn config_dir_rejects_shapes_outside_the_grammar() {
        let (root, _g) = platform_dirs_setup();

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
            let err = resolve_config_dir(declared).unwrap_err();
            assert!(err.contains(expected), "{declared} 应被拒绝：{err}");
        }
        assert!(
            !root.path().join("home").exists(),
            "被拒绝的声明不得产生任何目录"
        );
    }

    /// 逃逸校验看的是规范化后的真实路径：基准目录内的符号链接指向外面同样被拒。
    #[cfg(unix)]
    #[test]
    fn config_dir_rejects_a_symlink_escaping_its_base_dir() {
        let (root, _g) = platform_dirs_setup();
        let outside = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        fs::create_dir_all(&home).unwrap();
        std::os::unix::fs::symlink(outside.path(), home.join("link")).unwrap();

        let err = resolve_config_dir("$HOME/link").unwrap_err();

        assert!(err.contains("逃逸了基准目录"), "{err}");
    }

    /// 严格子代判定：基准目录内放一个指向基准目录自身的 symlink 即便路径段形态合法
    /// （不是 `.`、`..`、绝对路径），规范化后仍会落到基准目录本身——必须判逃逸。
    #[cfg(unix)]
    #[test]
    fn config_dir_rejects_a_path_that_canonicalizes_to_the_base_dir_itself() {
        let (root, _g) = platform_dirs_setup();
        let home = root.path().join("home");
        fs::create_dir_all(&home).unwrap();
        // `home/loop` 是指向 `home` 的 symlink。`$HOME/loop` 形态合法但规范化后
        // 就是基准目录本身——形态校验放行，逃逸校验必须兜住。
        std::os::unix::fs::symlink(&home, home.join("loop")).unwrap();

        let err = resolve_config_dir("$HOME/loop").unwrap_err();

        assert!(err.contains("逃逸了基准目录"), "{err}");
    }

    #[test]
    fn file_entry_must_be_a_relative_path_without_parent_dir() {
        let (_root, _g) = platform_dirs_setup();
        for entry in ["/etc/passwd", "../outside.wasm", "sub/../../outside.wasm"] {
            let err = parse_manifest(SourceKind::File, &manifest_text("pi", entry)).unwrap_err();
            assert!(err.contains("entry"), "entry {entry:?} 应被拒绝：{err}");
        }
        assert!(parse_manifest(SourceKind::File, &manifest_text("pi", "sub/p.wasm")).is_ok());
    }

    #[test]
    fn file_entry_rejects_non_https_schemes() {
        let (_root, _g) = platform_dirs_setup();
        // 带 scheme 的 entry 只接受 https；http:// 等既不是相对路径也不会联网获取，
        // 必须在解析期拒绝，而不是当成相对路径去读一个不可能存在的文件。
        for entry in [
            "http://example.com/plugin.wasm",
            "file:///tmp/plugin.wasm",
            "ftp://example.com/plugin.wasm",
        ] {
            let err = parse_manifest(SourceKind::File, &manifest_text("pi", entry)).unwrap_err();
            assert!(
                err.contains("entry") && err.contains("https URL"),
                "entry {entry:?} 应被拒绝：{err}"
            );
        }
        assert!(
            parse_manifest(
                SourceKind::File,
                &manifest_text("pi", "https://example.com/plugin.wasm")
            )
            .is_ok(),
            "file 来源的 https entry 合法（回源到网络）"
        );
    }

    #[test]
    fn https_entry_must_be_an_https_url() {
        let (_root, _g) = platform_dirs_setup();
        let ok = manifest_text("pi", "https://example.com/releases/download/v1/plugin.wasm");
        assert_eq!(
            parse_manifest(SourceKind::Https, &ok).unwrap().entry,
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
            let err = parse_manifest(SourceKind::Https, &manifest_text("pi", entry)).unwrap_err();
            assert!(
                err.contains("https URL"),
                "https 来源的 entry {entry:?} 应被拒绝：{err}"
            );
        }
    }

    #[test]
    fn builtin_entry_is_not_constrained() {
        let (_root, _g) = platform_dirs_setup();
        // 内置插件的 wasm 内嵌于二进制，entry 不参与解析。
        let text = manifest_text("pi", "target/wasm32-wasip2/release/maestro_plugin_pi.wasm");
        assert!(parse_manifest(SourceKind::Builtin, &text).is_ok());
    }

    #[test]
    fn is_https_url_accepts_only_absolute_https_urls() {
        assert!(is_https_url("https://example.com/manifest.json"));
        assert!(is_https_url("https://example.com:8443/a/b?c=d"));
        assert!(!is_https_url("http://example.com/manifest.json"));
        assert!(!is_https_url("file:///tmp/manifest.json"));
        assert!(!is_https_url("/tmp/manifest.json"));
        assert!(!is_https_url("https://"));
        assert!(!is_https_url(""));
    }

    #[test]
    fn source_kind_is_derived_from_the_source_string() {
        assert_eq!(
            SourceKind::from_source("builtin:pi"),
            Some(SourceKind::Builtin)
        );
        assert_eq!(
            SourceKind::from_source("https://example.com/manifest.json"),
            Some(SourceKind::Https)
        );
        assert_eq!(
            SourceKind::from_source("/tmp/plugin/manifest.json"),
            Some(SourceKind::File)
        );
        assert_eq!(
            SourceKind::from_source("plugins/pi/manifest.json"),
            None,
            "相对路径不是合法的来源身份（file 来源要求本机绝对路径）"
        );
        assert_eq!(SourceKind::from_source("file:///tmp/manifest.json"), None);
        assert_eq!(SourceKind::from_source("git://example.com/x.git"), None);
    }
}
