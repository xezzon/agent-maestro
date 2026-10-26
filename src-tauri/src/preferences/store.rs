//! 偏好存储：启动时从磁盘加载进内存，变更后原子写回。
//!
//! 与配置存储（[`crate::store`]）平行而独立：自己的内存状态、自己的锁、自己的
//! 错误状态机。两者刻意不共享——配置损坏要「拒写」（ADR 0001），偏好损坏要
//! 「兜底并重写」（ADR 0023），一套状态机同时表达不了这两条相反的策略。

use std::{
    fs,
    path::{Path, PathBuf},
    sync::RwLock,
};

use crate::{
    errors::{LockPoisoned, PreferencesError},
    fs_util::{Durability, write_atomic},
};

use super::{Loaded, PREFERENCES_VERSION, Preferences, PreferencesFile};

/// 偏好存储：启动时从磁盘加载进内存，变更后原子写回。
///
/// 内存中始终有一份值（读不出来时是兜底后的默认值），故读取只可能因锁不可用
/// 而失败。
pub(crate) struct PreferencesStore {
    /// 偏好文件路径。构造方显式传入：依赖在签名上可见，测试也能指向任意路径。
    path: PathBuf,
    /// 内存状态：读多写少（读取走读锁，整包更新走写锁）。
    state: RwLock<Preferences>,
    /// 那个文件归不归当前版本管：版本不受支持时只读不写。
    file_state: FileState,
}

/// 偏好文件是否归当前版本管。
///
/// 版本不受支持的文件可能来自更新版本：覆盖它等于用旧版销毁新版的数据
/// （ADR 0023），故那一份只读不写。
#[derive(Debug, Clone)]
enum FileState {
    /// 归当前版本管：可读写。
    Managed,
    /// 版本不受支持：只读不写（文件里的版本取值原样带进报错）。
    Foreign { found: String },
}

impl PreferencesStore {
    /// 从给定路径加载偏好。加载过程**不失败**：读不出来、损坏、版本不受支持
    /// 一律兜底成默认值，并按各自的规则处理磁盘（见 ADR 0023 的容错表）。
    pub(crate) fn new(path: PathBuf) -> Self {
        let (preferences, file_state) = load(&path);
        Self {
            path,
            state: RwLock::new(preferences),
            file_state,
        }
    }

    /// 当前偏好。
    pub(crate) fn get(&self) -> Result<Preferences, PreferencesError> {
        let guard = self.state.read().map_err(|_| LockPoisoned)?;
        Ok(guard.clone())
    }

    /// 整包替换偏好：强制版本号后原子写回磁盘，成功才更新内存状态。
    ///
    /// 写盘失败即报错，内存保持原值——界面据此回滚到切换前的值，不展示一个
    /// 没被保存的状态。
    ///
    /// 文件版本不受支持时整包替换被拒：那一份可能由更新版本写出，覆盖它等于
    /// 降级销毁（ADR 0023；用户故事 13 说的正是这件事）。
    pub(crate) fn update(&self, preferences: Preferences) -> Result<Preferences, PreferencesError> {
        if let FileState::Foreign { found } = &self.file_state {
            return Err(PreferencesError::unsupported_version(
                &self.path,
                found.clone(),
                PREFERENCES_VERSION,
            ));
        }
        // 写锁覆盖「落盘 + 换内存」两步：并发两次更新不能交错，否则先落盘的那次
        // 可能被后落盘的覆盖、内存却停在中间态。
        let mut guard = self.state.write().map_err(|_| LockPoisoned)?;
        let mut next = preferences;
        next.version = PREFERENCES_VERSION;
        persist(&self.path, &next)?;
        *guard = next.clone();
        Ok(next)
    }
}

/// 读入偏好并按容错表处理磁盘。
///
/// 容错表（ADR 0023）：
/// - 文件不存在 → 按默认值创建并写盘；
/// - 缺字段 / 主题值非法 → 兜底为默认值并写回修正；
/// - JSON 语法坏 → 原文件留档为 `preferences.json.bad`，再写默认值；
/// - 版本不受支持 → 按默认值运行，不读写该文件；
/// - 任一写回失败 → 只记 WARN，内存继续用兜底值，不阻断启动。
fn load(path: &Path) -> (Preferences, FileState) {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let preferences = Preferences::default();
            write_back(path, &preferences);
            return (preferences, FileState::Managed);
        }
        Err(err) => {
            // 读不出来（权限、路径不是文件……）：按兜底值运行，且不碰那个文件——
            // 读不动不等于可以改写它。
            log::warn!("read preferences failed: {err}");
            return (Preferences::default(), FileState::Managed);
        }
    };

    match serde_json::from_str::<PreferencesFile>(&text) {
        Ok(file) => match file.normalize() {
            Loaded::Usable {
                preferences,
                needs_rewrite,
            } => {
                if needs_rewrite {
                    write_back(path, &preferences);
                }
                (preferences, FileState::Managed)
            }
            Loaded::UnsupportedVersion { found } => {
                log::warn!(
                    "preferences version {found} is not supported (current: {PREFERENCES_VERSION}); using defaults and leaving the file untouched"
                );
                (Preferences::default(), FileState::Foreign { found })
            }
        },
        Err(err) => {
            // 语法坏（或根本不是一份偏好文档）：原文件留档后再写默认值，不静默
            // 销毁用户手写的内容。
            log::warn!("preferences file is corrupt: {err}");
            match archive(path) {
                Ok(backup) => {
                    log::warn!("corrupt preferences file archived as {}", backup.display());
                    write_back(path, &Preferences::default());
                }
                Err(err) => {
                    log::warn!("failed to archive the corrupt preferences file: {err}");
                }
            }
            (Preferences::default(), FileState::Managed)
        }
    }
}

/// 写回偏好：失败只记 WARN，不阻断启动（偏好是外观问题，不是启动阻塞）。
fn write_back(path: &Path, preferences: &Preferences) {
    if let Err(err) = persist(path, preferences) {
        log::warn!("write preferences failed: {err}");
    }
}

/// 原子写入：同目录临时文件 → 落盘 → rename 覆盖目标。
fn persist(path: &Path, preferences: &Preferences) -> Result<(), PreferencesError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir).map_err(|err| PreferencesError::io(path, err.to_string()))?;
    let json = serde_json::to_string_pretty(preferences)
        .map_err(|err| PreferencesError::serialize(err.to_string()))?;
    // 偏好没有第二来源：落盘前 fsync，保证文件内容已到设备（见 `crate::fs_util`）。
    write_atomic(path, format!("{json}\n").as_bytes(), Durability::Synced)
        .map_err(|err| PreferencesError::io(path, err.to_string()))
}

/// 把损坏的偏好文件留档为 `preferences.json.bad`（固定名，再次损坏覆盖同名
/// 留档），返回留档路径。
fn archive(path: &Path) -> std::io::Result<PathBuf> {
    let mut backup = path.as_os_str().to_owned();
    backup.push(".bad");
    let backup = PathBuf::from(backup);
    // rename 覆盖已存在的留档在 Unix 上直接成功、Windows 上会失败——先删旧的。
    if backup.exists() {
        fs::remove_file(&backup)?;
    }
    fs::rename(path, &backup)?;
    Ok(backup)
}
