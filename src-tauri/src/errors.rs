//! 错误类型：按领域拆分，每个类型自己排版面向用户的文案（[`Display`]）。
//!
//! 错误类型大多住在 crate 根而非某一层：配置存储与插件服务两侧都消费其中一部分
//! （插件服务经端口收存储错误，命令层把两侧的错误都转成字符串交给 IPC），
//! 放在中立层才不会让两侧互相依赖成环。路径解析失败（`paths::PathError`）例外：
//! 它只有一个消费者，随 [`crate::paths`] 住，`paths` 因此不反向依赖本模块。
//!
//! 存储错误按仓储职责拆开：[`ConfigError`]（配置文档读写）、[`ProviderError`]、
//! [`PluginEntryError`] 与 [`VariableError`]。各自把持久化失败包在 `Storage`
//! 变体里——调用方只关心「这次操作失败了」，无需在签名里同时列出领域错误与
//! 存储错误。插件服务另有 [`PluginError`]，它把来自端口的两类存储错误与自己的
//! 领域错误拼在一起。

use std::{
    fmt::{self, Display},
    path::PathBuf,
};

/// 配置存储的锁被毒化（其它命令持锁期间 panic）。
///
/// 这不是任何领域错误的一种：毒化的是 `AppStore` 的锁而非配置本身。锁边界
/// 经 [`From`] 把它折进调用方的错误类型——每个可经由锁触发的错误类型都有一段
/// `From<LockPoisoned>`，统一归到「配置存储不可用」这一句。
pub(crate) struct LockPoisoned;

/// 配置存储不可用时对用户可见的原因（锁毒化与保护态共用）。
const STORE_LOCK_POISONED: &str = "配置存储不可用";

/// 配置文件本身的错误：读取、解析、序列化与落盘。
#[derive(Debug, Clone)]
pub(crate) enum ConfigError {
    /// 配置文件存在但无法解析。
    Corrupt { path: PathBuf, detail: String },
    /// 配置文件版本不被当前应用支持。
    UnsupportedVersion {
        path: PathBuf,
        found: u32,
        /// 当前应用支持的版本。由存储层在比较时填入——错误类型不反向依赖
        /// [`crate::config`]（#141 无环约束）。
        supported: u32,
    },
    /// 读写配置文件时发生 IO 错误。
    Io { path: PathBuf, detail: String },
    /// 配置读得出来，却写不出合法的 JSON（序列化失败）。
    ///
    /// 与 IO 失败是两件事：文件没坏、也不是写不动，而是内容写不成 JSON——
    /// 混进 `Io` 会让报错指向错误的修复方向（去查磁盘权限，而不是查内容）。
    Serialize { detail: String },
    /// 配置存储的锁不可用（其它命令持锁期间 panic 毒化了锁）。
    Unavailable,
}

impl ConfigError {
    pub(crate) fn corrupt(path: &std::path::Path, detail: impl Into<String>) -> Self {
        Self::Corrupt {
            path: path.to_path_buf(),
            detail: detail.into(),
        }
    }

    pub(crate) fn io(path: &std::path::Path, detail: impl Into<String>) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            detail: detail.into(),
        }
    }

    pub(crate) fn serialize(detail: impl Into<String>) -> Self {
        Self::Serialize {
            detail: detail.into(),
        }
    }
}

impl From<LockPoisoned> for ConfigError {
    fn from(_: LockPoisoned) -> Self {
        Self::Unavailable
    }
}

impl Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Corrupt { path, detail } => write!(
                f,
                "配置文件已损坏：{}\n原因：{detail}\n请修复或删除该文件后重启应用；在此之前 Maestro 拒绝任何写入，绝不会静默重建。",
                path.display()
            ),
            ConfigError::UnsupportedVersion {
                path,
                found,
                supported,
            } => write!(
                f,
                "配置文件版本不受支持：{}\n文件中的 version 为 {found}，当前应用仅支持 {supported}。\n请修复该文件后重启应用；在此之前 Maestro 拒绝任何写入，绝不会静默重建。",
                path.display()
            ),
            ConfigError::Io { path, detail } => {
                write!(f, "读写配置文件失败：{}\n原因：{detail}", path.display())
            }
            ConfigError::Serialize { detail } => {
                write!(f, "序列化配置文件失败\n原因：{detail}")
            }
            ConfigError::Unavailable => write!(f, "{STORE_LOCK_POISONED}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Provider 仓储的错误：重复/缺失与插值校验失败。
#[derive(Debug, Clone)]
pub(crate) enum ProviderError {
    /// 已存在同名 Provider。
    DuplicateSlug { slug: String },
    /// Provider 不存在（update/delete 不做 upsert，绝不静默覆盖）。
    MissingSlug { slug: String },
    /// Provider 对（变更后的）全局变量表插值失败：变量未定义或占位符语法
    /// 错误。与投影时的插值失败同因同报，只是提前到落盘前拦截
    /// （provider↔variables 一致性，ADR 0015）。
    Interpolation(ProviderInterpolationError),
    /// 配置的读取或落盘失败。
    Storage(ConfigError),
}

impl ProviderError {
    pub(crate) fn duplicate_slug(slug: &str) -> Self {
        Self::DuplicateSlug {
            slug: slug.to_owned(),
        }
    }

    pub(crate) fn missing_slug(slug: &str) -> Self {
        Self::MissingSlug {
            slug: slug.to_owned(),
        }
    }
}

impl From<ConfigError> for ProviderError {
    fn from(error: ConfigError) -> Self {
        Self::Storage(error)
    }
}

impl From<LockPoisoned> for ProviderError {
    fn from(_: LockPoisoned) -> Self {
        Self::Storage(ConfigError::Unavailable)
    }
}

impl Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderError::DuplicateSlug { slug } => write!(f, "已存在同名 Provider：{slug}"),
            ProviderError::MissingSlug { slug } => write!(f, "Provider 不存在：{slug}"),
            ProviderError::Interpolation(error) => write!(f, "{error}"),
            ProviderError::Storage(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ProviderError {}

/// 插件条目仓储的错误：来源即条目唯一身份，id 是来源到落位目录的唯一映射。
#[derive(Debug, Clone)]
pub(crate) enum PluginEntryError {
    /// 插件条目不存在。
    MissingSource { source: String },
    /// 已存在同一来源的插件条目。
    DuplicateSource { source: String },
    /// 同一 id 已被其它来源的插件条目持有。
    DuplicateId { id: String, source: String },
    /// 配置的读取或落盘失败。
    Storage(ConfigError),
}

impl PluginEntryError {
    pub(crate) fn missing_source(source: &str) -> Self {
        Self::MissingSource {
            source: source.to_owned(),
        }
    }

    pub(crate) fn duplicate_source(source: &str) -> Self {
        Self::DuplicateSource {
            source: source.to_owned(),
        }
    }

    pub(crate) fn duplicate_id(id: &str, source: &str) -> Self {
        Self::DuplicateId {
            id: id.to_owned(),
            source: source.to_owned(),
        }
    }
}

impl From<ConfigError> for PluginEntryError {
    fn from(error: ConfigError) -> Self {
        Self::Storage(error)
    }
}

impl From<LockPoisoned> for PluginEntryError {
    fn from(_: LockPoisoned) -> Self {
        Self::Storage(ConfigError::Unavailable)
    }
}

impl Display for PluginEntryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginEntryError::MissingSource { source } => write!(f, "插件条目不存在：{source}"),
            PluginEntryError::DuplicateSource { source } => {
                write!(f, "已存在同一来源的插件条目：{source}")
            }
            PluginEntryError::DuplicateId { id, source } => {
                write!(f, "插件 id「{id}」已被来源 {source} 占用")
            }
            PluginEntryError::Storage(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for PluginEntryError {}

/// 全局变量表的错误：名合法性，以及换表后对存量 Provider 的插值复检。
#[derive(Debug, Clone)]
pub(crate) enum VariableError {
    /// 变量名不合法：占位符无法引用（变量名语法见 ADR 0014）。
    InvalidVariableName { name: String },
    /// 换表后某个 Provider 的插值失败（删除在用变量即整表拒绝）。
    Interpolation(ProviderInterpolationError),
    /// 配置的读取或落盘失败。
    Storage(ConfigError),
}

impl VariableError {
    pub(crate) fn invalid_variable_name(name: &str) -> Self {
        Self::InvalidVariableName {
            name: name.to_owned(),
        }
    }
}

impl From<ConfigError> for VariableError {
    fn from(error: ConfigError) -> Self {
        Self::Storage(error)
    }
}

impl From<LockPoisoned> for VariableError {
    fn from(_: LockPoisoned) -> Self {
        Self::Storage(ConfigError::Unavailable)
    }
}

impl Display for VariableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VariableError::InvalidVariableName { name } => write!(
                f,
                "变量名不合法：「{name}」\n变量名只能包含字母、数字与下划线（见 ADR 0014）。"
            ),
            VariableError::Interpolation(error) => write!(f, "{error}"),
            VariableError::Storage(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for VariableError {}

/// 占位符插值失败：底层库的报错原文（只含变量名与位置，不含值与密钥）。
#[derive(Debug, Clone)]
pub(crate) struct InterpolationError {
    detail: String,
}

impl InterpolationError {
    pub(crate) fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl Display for InterpolationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.detail)
    }
}

impl std::error::Error for InterpolationError {}

/// 插值失败：字段名 + 原因。
///
/// 字段名指明插的是哪个字段（`base_url.openai-completions`、`models[1].display_name`），
/// 原因取自库（只含变量名与位置，不含值与密钥）。
///
/// 用具名字段而非匿名元组：元组的两个位置写反了编译器不报错，解构点读到的
/// 字段名与原因会悄悄对调。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FieldError {
    /// 出错的字段名。
    pub(crate) field: String,
    /// 原因（库报错）。
    pub(crate) reason: String,
}

/// 跨 Provider 插值失败：出错的条目（slug）与其字段级失败。
///
/// 面向用户的句式由本类型独家产出（见 [`Display`]）：slug + 字段名 + 原因入句。
/// 落盘前拦截（[`ProviderError::Interpolation`] / [`VariableError::Interpolation`]
/// 包裹本类型）与投影时拦截（调用方直接格式化本类型）据此同因同报、逐字一致
/// ——同一句式只有这一处实现。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderInterpolationError {
    /// 出错的 Provider（slug）。
    pub(crate) slug: String,
    /// 字段级失败：字段名 + 库报错。
    pub(crate) error: FieldError,
}

impl Display for ProviderInterpolationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Provider「{}」的 {} 插值失败：{}",
            self.slug, self.error.field, self.error.reason
        )
    }
}

impl std::error::Error for ProviderInterpolationError {}

/// 日志文件目标的写探测失败；调用方据此降级为只装 Stdout。
#[derive(Debug, Clone)]
pub(crate) enum LoggingError {
    CreateDir { path: PathBuf, detail: String },
    OpenFile { path: PathBuf, detail: String },
}

impl LoggingError {
    pub(crate) fn create_dir(path: &std::path::Path, detail: impl Into<String>) -> Self {
        Self::CreateDir {
            path: path.to_path_buf(),
            detail: detail.into(),
        }
    }

    pub(crate) fn open_file(path: &std::path::Path, detail: impl Into<String>) -> Self {
        Self::OpenFile {
            path: path.to_path_buf(),
            detail: detail.into(),
        }
    }
}

impl Display for LoggingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoggingError::CreateDir { path, detail } => {
                write!(f, "create {}: {detail}", path.display())
            }
            LoggingError::OpenFile { path, detail } => {
                write!(f, "open {}: {detail}", path.display())
            }
        }
    }
}

impl std::error::Error for LoggingError {}

/// https 拉取的错误：网络、状态码、响应体体积。
#[derive(Debug, Clone)]
pub(crate) enum FetchError {
    /// 客户端初始化失败（TLS 后端不可用）。
    ClientInit { detail: String },
    /// 请求失败（连接、超时或 4xx/5xx 状态码）。
    Request { url: String, detail: String },
    /// 读取响应体失败。
    ReadResponse { url: String, detail: String },
    /// 响应超过体积上限（不静默截断）。
    TooLarge { url: String, limit_mib: u64 },
}

impl FetchError {
    pub(crate) fn client_init(detail: impl Into<String>) -> Self {
        Self::ClientInit {
            detail: detail.into(),
        }
    }

    pub(crate) fn request(url: &str, detail: impl Into<String>) -> Self {
        Self::Request {
            url: url.to_owned(),
            detail: detail.into(),
        }
    }

    pub(crate) fn read_response(url: &str, detail: impl Into<String>) -> Self {
        Self::ReadResponse {
            url: url.to_owned(),
            detail: detail.into(),
        }
    }

    pub(crate) fn too_large(url: &str, limit_mib: u64) -> Self {
        Self::TooLarge {
            url: url.to_owned(),
            limit_mib,
        }
    }
}

impl Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::ClientInit { detail } => {
                write!(f, "初始化 https 客户端失败：{detail}")
            }
            FetchError::Request { url, detail } => write!(f, "请求 {url} 失败：{detail}"),
            FetchError::ReadResponse { url, detail } => {
                write!(f, "读取 {url} 的响应失败：{detail}")
            }
            FetchError::TooLarge { url, limit_mib } => {
                write!(f, "{url} 的响应超过 {limit_mib} MiB 上限")
            }
        }
    }
}

impl std::error::Error for FetchError {}

/// 插件服务的错误：来源、manifest、组件、落位、插件配置与投影。
///
/// 插件服务经端口读写配置存储，故端口的两类错误（[`PluginEntryError`] /
/// [`ConfigError`]）也在这里汇合——调用方只看到一次操作失败，不必知道失败
/// 发生在服务层还是存储层。
#[derive(Debug, Clone)]
pub(crate) enum PluginError {
    // ---- 来源 ----
    UnrecognizedSource {
        source: String,
    },
    UnknownBuiltin {
        source: String,
    },

    // ---- 落位产物与目录 ----
    ReadFile {
        path: PathBuf,
        detail: String,
    },
    CreateDirectory {
        detail: String,
    },
    WriteFile {
        detail: String,
    },
    RemoveDirectory {
        path: PathBuf,
        detail: String,
    },

    // ---- 锁 ----
    RegistryUnavailable,
    InstallLockUnavailable,

    // ---- 生命周期 ----
    BuiltinNotRemovable {
        source: String,
    },
    IdChanged {
        previous: String,
        next: String,
    },
    CleanupFailed {
        error: String,
        cleanup: String,
        path: PathBuf,
    },

    // ---- 获取 ----
    Fetch(FetchError),
    DownloadManifest {
        detail: String,
    },
    ManifestNotUtf8 {
        detail: String,
    },

    // ---- manifest 校验 ----
    InvalidManifest {
        detail: String,
    },
    InvalidPluginId {
        id: String,
    },
    EmptyEntry,
    HttpsEntryRequired {
        entry: String,
    },
    RelativeOrHttpsEntryRequired {
        entry: String,
    },

    // ---- config_dir 声明与解析 ----
    ConfigDirMissing,
    ConfigDirVariableRequired {
        declared: String,
    },
    ConfigDirUnsupportedVariable {
        declared: String,
        variable: String,
    },
    ConfigDirMissingRelative {
        declared: String,
    },
    ConfigDirUnsafeRelative {
        declared: String,
    },
    ConfigDirCreate {
        detail: String,
    },
    ConfigDirResolveBase {
        detail: String,
    },
    ConfigDirResolve {
        detail: String,
    },
    ConfigDirEscape {
        declared: String,
    },
    ConfigDirShape,
    PlatformKeyUnsupported {
        key: String,
    },
    PlatformValueNotString {
        key: String,
    },
    PlatformMissing {
        platform: String,
    },

    // ---- 组件 ----
    InvalidComponent {
        detail: String,
    },
    IncompatibleInterface {
        detail: String,
    },
    WasiInit {
        detail: String,
    },
    PluginImportInit {
        detail: String,
    },
    OpenConfigDir {
        detail: String,
    },
    SetFuel {
        detail: String,
    },
    CallPlugin {
        detail: String,
    },
    PluginReturned {
        detail: String,
    },

    // ---- 插件配置 ----
    ReadPluginConfig {
        path: PathBuf,
        detail: String,
    },
    CorruptPluginConfig {
        path: PathBuf,
        detail: String,
    },
    SerializePluginConfig {
        detail: String,
    },
    SerializeForm {
        detail: String,
    },
    UndeclaredVariable {
        name: String,
    },

    // ---- 投影 ----
    PathOutsideConfigDir {
        file: String,
    },
    Interpolation(ProviderInterpolationError),

    // ---- 端口 ----
    StoreEntry(PluginEntryError),
    StoreConfig(ConfigError),
}

impl PluginError {
    pub(crate) fn read_file(path: &std::path::Path, detail: impl Into<String>) -> Self {
        Self::ReadFile {
            path: path.to_path_buf(),
            detail: detail.into(),
        }
    }

    pub(crate) fn write_file(detail: impl Into<String>) -> Self {
        Self::WriteFile {
            detail: detail.into(),
        }
    }

    pub(crate) fn remove_directory(path: &std::path::Path, detail: impl Into<String>) -> Self {
        Self::RemoveDirectory {
            path: path.to_path_buf(),
            detail: detail.into(),
        }
    }
}

impl From<FetchError> for PluginError {
    fn from(error: FetchError) -> Self {
        Self::Fetch(error)
    }
}

impl From<PluginEntryError> for PluginError {
    fn from(error: PluginEntryError) -> Self {
        Self::StoreEntry(error)
    }
}

impl From<ConfigError> for PluginError {
    fn from(error: ConfigError) -> Self {
        Self::StoreConfig(error)
    }
}

impl From<ProviderInterpolationError> for PluginError {
    fn from(error: ProviderInterpolationError) -> Self {
        Self::Interpolation(error)
    }
}

impl Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginError::UnrecognizedSource { source } => write!(
                f,
                "无法识别的插件来源：{source}（支持内置 builtin:<id>、指向 manifest.json 的 https 地址或本机绝对路径）"
            ),
            PluginError::UnknownBuiltin { source } => write!(f, "未知的内置插件来源：{source}"),
            PluginError::ReadFile { path, detail } => {
                write!(f, "读取 {} 失败：{detail}", path.display())
            }
            PluginError::CreateDirectory { detail } => write!(f, "创建插件目录失败：{detail}"),
            PluginError::WriteFile { detail } => write!(f, "{detail}"),
            PluginError::RemoveDirectory { path, detail } => {
                write!(f, "删除插件目录 {} 失败：{detail}", path.display())
            }
            PluginError::RegistryUnavailable => write!(f, "插件注册表不可用"),
            PluginError::InstallLockUnavailable => write!(f, "插件安装锁不可用"),
            PluginError::BuiltinNotRemovable { source } => {
                write!(f, "内置插件不可移除：{source}")
            }
            PluginError::IdChanged { previous, next } => write!(
                f,
                "上游 manifest 的插件 id 已从「{previous}」变更为「{next}」，拒绝重新加载"
            ),
            PluginError::CleanupFailed {
                error,
                cleanup,
                path,
            } => write!(
                f,
                "{error}；落位目录清理失败：{cleanup}（残留路径：{}）",
                path.display()
            ),
            PluginError::Fetch(error) => write!(f, "{error}"),
            PluginError::DownloadManifest { detail } => write!(f, "下载 manifest 失败：{detail}"),
            PluginError::ManifestNotUtf8 { detail } => {
                write!(f, "manifest.json 不是合法 UTF-8：{detail}")
            }
            PluginError::InvalidManifest { detail } => write!(f, "manifest.json 不合法：{detail}"),
            PluginError::InvalidPluginId { id } => write!(
                f,
                "manifest.json 不合法：插件 id「{id}」需以小写字母开头，仅允许小写字母、数字、连字符和下划线"
            ),
            PluginError::EmptyEntry => write!(f, "manifest.json 不合法：entry 不能为空"),
            PluginError::HttpsEntryRequired { entry } => write!(
                f,
                "manifest.json 不合法：https 来源的 entry「{entry}」必须是 https URL"
            ),
            PluginError::RelativeOrHttpsEntryRequired { entry } => write!(
                f,
                "manifest.json 不合法：entry「{entry}」必须是插件目录内的相对路径或 https URL"
            ),
            PluginError::ConfigDirMissing => write!(f, "config_dir 未配置"),
            PluginError::ConfigDirVariableRequired { declared } => write!(
                f,
                "config_dir「{declared}」必须以平台变量开头（支持：{}）",
                supported_variables()
            ),
            PluginError::ConfigDirUnsupportedVariable { declared, variable } => write!(
                f,
                "config_dir「{declared}」使用了不支持的平台变量「{variable}」（支持：{}）",
                supported_variables()
            ),
            PluginError::ConfigDirMissingRelative { declared } => {
                write!(f, "config_dir「{declared}」缺少相对片段")
            }
            PluginError::ConfigDirUnsafeRelative { declared } => write!(
                f,
                "config_dir「{declared}」的相对片段不得是绝对路径，也不得包含「..」或「.」"
            ),
            PluginError::ConfigDirCreate { detail } => {
                write!(f, "创建插件配置目录失败：{detail}")
            }
            PluginError::ConfigDirResolveBase { detail } => {
                write!(f, "解析平台基准目录失败：{detail}")
            }
            PluginError::ConfigDirResolve { detail } => {
                write!(f, "解析插件配置目录失败：{detail}")
            }
            PluginError::ConfigDirEscape { declared } => {
                write!(f, "config_dir「{declared}」逃逸了基准目录")
            }
            PluginError::ConfigDirShape => {
                write!(f, "config_dir 必须是字符串或按平台键声明的对象")
            }
            PluginError::PlatformKeyUnsupported { key } => write!(
                f,
                "config_dir 对象使用了不支持的平台键「{key}」（支持：linux、macos、windows）"
            ),
            PluginError::PlatformValueNotString { key } => {
                write!(f, "config_dir 对象的平台键「{key}」的值必须是字符串")
            }
            PluginError::PlatformMissing { platform } => {
                write!(f, "config_dir 对象缺少平台「{platform}」的声明")
            }
            PluginError::InvalidComponent { detail } => {
                write!(f, "不是有效的 WASM 组件：{detail}")
            }
            PluginError::IncompatibleInterface { detail } => {
                write!(f, "插件接口不兼容：{detail}")
            }
            PluginError::WasiInit { detail } => {
                write!(f, "初始化 WASI 宿主环境失败：{detail}")
            }
            PluginError::PluginImportInit { detail } => {
                write!(f, "初始化插件 import 失败：{detail}")
            }
            PluginError::OpenConfigDir { detail } => {
                write!(f, "无法开放插件配置目录：{detail}")
            }
            PluginError::SetFuel { detail } => {
                write!(f, "设置插件执行预算失败：{detail}")
            }
            PluginError::CallPlugin { detail } => write!(f, "调用插件失败：{detail}"),
            PluginError::PluginReturned { detail } => write!(f, "插件返回错误：{detail}"),
            PluginError::ReadPluginConfig { path, detail } => {
                write!(f, "读取插件配置失败：{}\n原因：{detail}", path.display())
            }
            PluginError::CorruptPluginConfig { path, detail } => write!(
                f,
                "插件配置已损坏：{}\n原因：{detail}\n请修复或删除该文件后重试；在此之前该插件的配置不可用。",
                path.display()
            ),
            PluginError::SerializePluginConfig { detail } => {
                write!(f, "序列化插件配置失败：{detail}")
            }
            PluginError::SerializeForm { detail } => write!(f, "序列化插件 form 失败：{detail}"),
            PluginError::UndeclaredVariable { name } => write!(
                f,
                "变量「{name}」未在全局变量表中声明，不能写入插件配置的覆盖项；请先在全局变量中声明。"
            ),
            PluginError::PathOutsideConfigDir { file } => {
                write!(
                    f,
                    "插件返回的文件路径「{file}」不是 config_dir 内的相对路径"
                )
            }
            PluginError::Interpolation(error) => write!(f, "{error}"),
            PluginError::StoreEntry(error) => write!(f, "{error}"),
            PluginError::StoreConfig(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for PluginError {}

#[cfg(test)]
impl PluginError {
    /// 测试辅助：错误文案是否包含某段文字（断言面向用户的句式）。
    pub(crate) fn contains(&self, pattern: &str) -> bool {
        self.to_string().contains(pattern)
    }
}

/// 声明里支持的平台变量前缀（错误信息里列给用户）。
fn supported_variables() -> String {
    crate::paths::SUPPORTED_VARIABLES.join("、")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// 落盘前拦截的插值失败文案逐字断言：slug、字段名、原因三者都在句子里，
    /// 且与投影时拦截同一句式（ADR 0015「报错与投影时同因同报」；句式由
    /// [`ProviderInterpolationError`] 独家产出）。
    #[test]
    fn interpolation_error_message_is_verbatim() {
        let error = ProviderInterpolationError {
            slug: "gateway".to_owned(),
            error: FieldError {
                field: "base_url.openai-completions".to_owned(),
                reason: "No such variable: $HOST".to_owned(),
            },
        };

        assert_eq!(
            ProviderError::Interpolation(error.clone()).to_string(),
            "Provider「gateway」的 base_url.openai-completions 插值失败：No such variable: $HOST"
        );
        assert_eq!(
            VariableError::Interpolation(error).to_string(),
            "Provider「gateway」的 base_url.openai-completions 插值失败：No such variable: $HOST"
        );
    }

    /// 保护态文案逐字钉住：路径、原因与修复指引三段排版都在。
    #[test]
    fn corrupt_error_message_is_verbatim() {
        let err = ConfigError::Corrupt {
            path: PathBuf::from("/home/u/.maestro/config.json"),
            detail: "expected value at line 1 column 1".to_owned(),
        };

        assert_eq!(
            err.to_string(),
            "配置文件已损坏：/home/u/.maestro/config.json\n原因：expected value at line 1 column 1\n请修复或删除该文件后重启应用；在此之前 Maestro 拒绝任何写入，绝不会静默重建。"
        );
    }

    #[test]
    fn unsupported_version_error_message_is_verbatim() {
        let err = ConfigError::UnsupportedVersion {
            path: PathBuf::from("/home/u/.maestro/config.json"),
            found: 99,
            supported: 1,
        };

        assert_eq!(
            err.to_string(),
            "配置文件版本不受支持：/home/u/.maestro/config.json\n文件中的 version 为 99，当前应用仅支持 1。\n请修复该文件后重启应用；在此之前 Maestro 拒绝任何写入，绝不会静默重建。"
        );
    }

    #[test]
    fn io_error_message_is_verbatim() {
        let err = ConfigError::Io {
            path: PathBuf::from("/home/u/.maestro/config.json"),
            detail: "Permission denied (os error 13)".to_owned(),
        };

        assert_eq!(
            err.to_string(),
            "读写配置文件失败：/home/u/.maestro/config.json\n原因：Permission denied (os error 13)"
        );
    }

    #[test]
    fn serialize_error_message_is_verbatim() {
        let err = ConfigError::Serialize {
            detail: "serializer error".to_owned(),
        };

        assert_eq!(
            err.to_string(),
            "序列化配置文件失败\n原因：serializer error"
        );
    }

    #[test]
    fn slug_and_source_error_messages_are_verbatim() {
        assert_eq!(
            ProviderError::DuplicateSlug {
                slug: "foo".to_owned()
            }
            .to_string(),
            "已存在同名 Provider：foo"
        );
        assert_eq!(
            ProviderError::MissingSlug {
                slug: "ghost".to_owned()
            }
            .to_string(),
            "Provider 不存在：ghost"
        );
        assert_eq!(
            PluginEntryError::MissingSource {
                source: "builtin:pi".to_owned()
            }
            .to_string(),
            "插件条目不存在：builtin:pi"
        );
        assert_eq!(
            PluginEntryError::DuplicateSource {
                source: "https://example.com/manifest.json".to_owned()
            }
            .to_string(),
            "已存在同一来源的插件条目：https://example.com/manifest.json"
        );
        assert_eq!(
            PluginEntryError::DuplicateId {
                id: "pi".to_owned(),
                source: "builtin:pi".to_owned()
            }
            .to_string(),
            "插件 id「pi」已被来源 builtin:pi 占用"
        );
    }

    #[test]
    fn invalid_variable_name_error_message_is_verbatim() {
        let err = VariableError::InvalidVariableName {
            name: "a-b".to_owned(),
        };

        assert_eq!(
            err.to_string(),
            "变量名不合法：「a-b」\n变量名只能包含字母、数字与下划线（见 ADR 0014）。"
        );
    }

    /// 锁毒化折进各错误域后仍是同一句文案：毒化的是锁，不是配置。
    #[test]
    fn lock_poisoned_folds_into_the_same_reason_for_every_error_domain() {
        assert_eq!(
            ConfigError::from(LockPoisoned).to_string(),
            "配置存储不可用"
        );
        assert_eq!(
            ProviderError::from(LockPoisoned).to_string(),
            "配置存储不可用"
        );
        assert_eq!(
            PluginEntryError::from(LockPoisoned).to_string(),
            "配置存储不可用"
        );
        assert_eq!(
            VariableError::from(LockPoisoned).to_string(),
            "配置存储不可用"
        );
    }
}
