//! 全局路径：主目录、平台基准目录与 Maestro 自己的目录布局。
//!
//! 此前这里是两个各自独立的全局单例：`Paths`（只持有主目录）与
//! `plugin::manifest::PlatformDirs`（持有主目录与四个平台基准目录）。两者各自
//! 初始化、各自失败（一个 panic、一个返回 `Err`），主目录还被解析了两遍，改
//! 一次主目录规则要改两处。现在合成一处：一个 [`Paths`] 同时持有主目录与四个
//! 平台基准目录（主目录兼作 `$HOME` 的落点，五个变量各有一个基准目录），由
//! `lib.rs` 的 `run()` 在 attach logger 之前一次性初始化，其余地方只读。
//!
//! 平台基准目录是 `config_dir` 声明里变量前缀的落点（ADR 0011）：本模块只回答
//! 「这个变量对应哪个目录」，声明的语法与逃逸校验归 `plugin::manifest`。
//!
//! 失败策略（全仓统一，此前是两个单例各说各的）：环境不可用（主目录或平台目录
//! 解析不出来）由 [`Paths::init_from_system`] 返回 `Err`，调用方（应用 `run()`，
//! GTK/WebKit 初始化之前）让启动以失败退出；全局尚未初始化是程序错误——应用
//! `run()` 必定先初始化，测试夹具亦然——由 [`Paths::get`] panic 并给出明确消息。

use std::path::{Path, PathBuf};

use crate::{errors::PathError, test_global::Global};

#[cfg(test)]
use crate::test_global::TestGuard;

/// 全局路径：生产期进程内一份，测试期每线程一份（见 [`crate::test_global`]）。
static PATHS: Global<Paths> = Global::new();

/// 声明里支持的平台变量前缀（ADR 0011）：五个，跨平台固定。
///
/// 语法检查（纯判断，不读目录）据此判断声明的变量前缀是否受支持；
/// [`Paths::base_dir`] 负责把受支持的变量映射到各自的基准目录。
pub(crate) const SUPPORTED_VARIABLES: [&str; 5] = [
    "$HOME",
    "$XDG_CONFIG_HOME",
    "$LOCAL_APP_CONFIG",
    "$XDG_DATA_HOME",
    "$LOCAL_APP_DATA",
];

/// 宿主路径：主目录 + 四个平台基准目录（主目录兼作 `$HOME` 的落点）。
///
/// 五个基准目录此前由调用方各拼一遍——参数一多就容易拼错位（构造函数的五个
/// 参数全是 `PathBuf`，编译器分不清谁是谁）。派生规则收在本模块一处，构造只走
/// [`Paths::init_from_system`]（生产）与三个测试构造器。
#[derive(Clone, Debug)]
pub(crate) struct Paths {
    home: PathBuf,
    /// `$XDG_CONFIG_HOME` 的基准目录。
    config: PathBuf,
    /// `$LOCAL_APP_CONFIG` 的基准目录。
    config_local: PathBuf,
    /// `$XDG_DATA_HOME` 的基准目录。
    data: PathBuf,
    /// `$LOCAL_APP_DATA` 的基准目录。
    data_local: PathBuf,
}

impl Paths {
    /// 生产环境初始化全局实例：一次性解析宿主目录（主目录 + 四个平台基准目录）。
    /// 任一项不可用即报错。重复调用无效（以首次为准）。
    ///
    /// 测试构建下不读真实环境——全局实例由测试夹具注入（见 crate 级 `testutil`
    /// 模块），故这里是空操作：测试若漏了注入，由 [`Paths::get`] 报错，而不是静默落到
    /// 真实目录。
    pub(crate) fn init_from_system() -> Result<(), PathError> {
        #[cfg(not(test))]
        {
            let missing = PathError::unavailable;
            PATHS.set(Self {
                home: dirs::home_dir().ok_or_else(|| missing("主目录"))?,
                config: dirs::config_dir().ok_or_else(|| missing("配置目录"))?,
                config_local: dirs::config_local_dir().ok_or_else(|| missing("本地配置目录"))?,
                data: dirs::data_dir().ok_or_else(|| missing("数据目录"))?,
                data_local: dirs::data_local_dir().ok_or_else(|| missing("本地数据目录"))?,
            });
        }
        Ok(())
    }

    /// 全部基准目录都落在同一个根之下（测试用）。
    ///
    /// 只有「验证每个平台变量映射到自己的基准目录」时需要这种布局——五个基准
    /// 目录（含主目录）若指向同一处，就分不清哪个变量解析到了哪里。
    #[cfg(test)]
    pub(crate) fn from_root(root: &Path) -> Self {
        Self {
            home: root.to_path_buf(),
            config: root.to_path_buf(),
            config_local: root.to_path_buf(),
            data: root.to_path_buf(),
            data_local: root.to_path_buf(),
        }
    }

    /// 主目录 + 平台约定的子目录（测试用）。
    #[cfg(test)]
    pub(crate) fn from_home(home: &Path) -> Self {
        Self {
            home: home.to_path_buf(),
            config: home.join(".config"),
            config_local: home.join(".config"),
            data: home.join(".local/share"),
            data_local: home.join(".local/share"),
        }
    }

    /// 五个基准目录（含主目录）互不相同（各自是根下的一个具名子目录，测试用）。
    #[cfg(test)]
    pub(crate) fn distinct_under(root: &Path) -> Self {
        Self {
            home: root.join("home"),
            config: root.join("config"),
            config_local: root.join("config-local"),
            data: root.join("data"),
            data_local: root.join("data-local"),
        }
    }

    /// 获取全局实例（值语义，clone 成本极低）。
    ///
    /// 尚未初始化即 panic：这是程序错误而非运行时状态——应用 `run()` 阶段必定
    /// 先初始化（[`Paths::init_from_system`]），测试夹具亦然。
    pub(crate) fn get() -> Self {
        PATHS
            .get()
            .expect("全局路径尚未初始化：调用 Paths::init_from_system 后再使用")
    }

    /// 测试辅助：设置当前线程的全局实例，返回 RAII guard（drop 时自动清理）。
    #[cfg(test)]
    pub(crate) fn test_guard(paths: Self) -> TestGuard<Paths> {
        Self::init_test(paths);
        TestGuard::new(&PATHS)
    }

    /// 测试辅助：直接设置当前线程的全局实例（不返回 guard，自行管理生命周期）。
    #[cfg(test)]
    pub(crate) fn init_test(paths: Self) {
        PATHS.set(paths);
    }

    /// Maestro 自己的目录：`~/.maestro`。
    pub(crate) fn maestro_dir(&self) -> PathBuf {
        self.home.join(".maestro")
    }

    /// 配置文件路径：`~/.maestro/config.json`。
    pub(crate) fn config_path(&self) -> PathBuf {
        self.maestro_dir().join("config.json")
    }

    /// 偏好文件路径：`~/.maestro/preferences.json`（ADR 0023）。
    ///
    /// 与 [`Paths::config_path`] 并列但不是同一份数据：偏好是应用级用户偏好
    /// （主题等），独立于领域配置（Provider、变量、插件），容错策略也相反
    /// （偏好兜底并重写，配置损坏即拒写）。两条路径由同一处派生，将来若要把
    /// `~/.maestro` 的布局收成一份文档，只改这里。
    pub(crate) fn preferences_path(&self) -> PathBuf {
        self.maestro_dir().join("preferences.json")
    }

    /// 插件落位目录的父目录：`~/.maestro/plugins`。
    pub(crate) fn plugins_dir(&self) -> PathBuf {
        self.maestro_dir().join("plugins")
    }

    /// 日志目录（ADR 0008）：`~/.maestro/logs`，用户可见、易备份、跨平台统一。
    pub(crate) fn logs_dir(&self) -> PathBuf {
        self.maestro_dir().join("logs")
    }

    /// 单个插件的落位目录：`~/.maestro/plugins/<id>`。
    pub(crate) fn plugin_dir(&self, plugin_id: &str) -> PathBuf {
        self.plugins_dir().join(plugin_id)
    }

    /// 平台变量前缀 → 基准目录；不支持的变量返回 `None`。
    ///
    /// `~` 与 `$HOME` 同义：两者解析到同一基准目录（ADR 0011）。
    pub(crate) fn base_dir(&self, variable: &str) -> Option<&Path> {
        match variable {
            "~" | "$HOME" => Some(&self.home),
            "$XDG_CONFIG_HOME" => Some(&self.config),
            "$LOCAL_APP_CONFIG" => Some(&self.config_local),
            "$XDG_DATA_HOME" => Some(&self.data),
            "$LOCAL_APP_DATA" => Some(&self.data_local),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_guard_sets_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        {
            let _g = Paths::test_guard(Paths::from_home(dir.path()));
            let paths = Paths::get();
            assert_eq!(paths.maestro_dir(), dir.path().join(".maestro"));
            assert_eq!(paths.config_path(), dir.path().join(".maestro/config.json"));
        }
        // guard drop 后应清理
        assert!(PATHS.get().is_none());
    }

    /// 平台变量前缀各落各的基准目录：`base_dir` 是声明语法的落点，
    /// 映射错位会让 `config_dir` 解析到别的目录（ADR 0011）。
    #[test]
    fn base_dir_maps_each_platform_variable_to_its_own_base() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::distinct_under(root.path());

        assert_eq!(
            paths.base_dir("$HOME"),
            Some(root.path().join("home").as_path())
        );
        assert_eq!(
            paths.base_dir("$XDG_CONFIG_HOME"),
            Some(root.path().join("config").as_path())
        );
        assert_eq!(
            paths.base_dir("$LOCAL_APP_CONFIG"),
            Some(root.path().join("config-local").as_path())
        );
        assert_eq!(
            paths.base_dir("$XDG_DATA_HOME"),
            Some(root.path().join("data").as_path())
        );
        assert_eq!(
            paths.base_dir("$LOCAL_APP_DATA"),
            Some(root.path().join("data-local").as_path())
        );
        assert_eq!(paths.base_dir("$FOO"), None);
    }

    /// 受支持的变量前缀与基准目录的映射一一对应：语法检查（纯判断）认这份
    /// 清单，解析（读目录）认 [`Paths::base_dir`]，两者漂移会让同一份声明在
    /// 解析期通过、在落位期才失败。
    #[test]
    fn every_supported_variable_maps_to_a_base_dir() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::distinct_under(root.path());

        for variable in SUPPORTED_VARIABLES {
            assert!(
                paths.base_dir(variable).is_some(),
                "{variable} 是受支持的变量前缀，应当有基准目录"
            );
        }
        assert_eq!(
            SUPPORTED_VARIABLES.len(),
            5,
            "受支持的变量前缀是固定五个（ADR 0011）"
        );
    }
}
