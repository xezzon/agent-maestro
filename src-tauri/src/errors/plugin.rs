//! 插件服务的错误：来源、manifest、组件、落位、插件配置与投影。

use std::{
    fmt::{self, Display},
    path::{Path, PathBuf},
};

use super::{
    config::ConfigError, entry::PluginEntryError, fetch::FetchError,
    interpolation::ProviderInterpolationError,
};

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
        /// 受支持的平台变量前缀（由调用方填入：错误类型不反向依赖 `paths`）。
        supported: &'static [&'static str],
    },
    ConfigDirUnsupportedVariable {
        declared: String,
        variable: String,
        /// 受支持的平台变量前缀（由调用方填入：错误类型不反向依赖 `paths`）。
        supported: &'static [&'static str],
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
    pub(crate) fn read_file(path: &Path, detail: impl Into<String>) -> Self {
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

    pub(crate) fn remove_directory(path: &Path, detail: impl Into<String>) -> Self {
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
            PluginError::ConfigDirVariableRequired {
                declared,
                supported,
            } => write!(
                f,
                "config_dir「{declared}」必须以平台变量开头（支持：{}）",
                supported.join("、")
            ),
            PluginError::ConfigDirUnsupportedVariable {
                declared,
                variable,
                supported,
            } => write!(
                f,
                "config_dir「{declared}」使用了不支持的平台变量「{variable}」（支持：{}）",
                supported.join("、")
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
