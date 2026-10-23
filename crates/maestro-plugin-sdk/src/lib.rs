//! `maestro-plugin-sdk`：Maestro 插件接口合同 `maestro:plugin` 的类型化 Rust 绑定。
//!
//! 插件作者只需依赖本 crate，实现 re-export 的 [`Guest`] trait，再以
//! [`export!`] 声明导出类型即可构建 wasm 组件，无需 vendor WIT 文件：
//!
//! ```ignore
//! use maestro_plugin_sdk::{export, Guest, Provider};
//!
//! struct MyPlugin;
//!
//! impl Guest for MyPlugin {
//!     fn write_providers(providers: Vec<Provider>) -> Result<Vec<String>, String> {
//!         // 把 Provider 投影进插件 manifest 声明的 config_dir（预开放为 "/"）。
//!         # Ok(Vec::new())
//!     }
//! }
//!
//! export!(MyPlugin);
//! ```
//!
//! # 过程性日志
//!
//! 投影期间需要向用户外显的非致命事件（逐条容错跳过、降级提示等）经
//! [`log`] 上报（`maestro:plugin` v1.1.0 起，见 issue #82）：
//!
//! ```ignore
//! use maestro_plugin_sdk::{log, Level};
//!
//! log(Level::Warning, "模型 x 已弃用，仍继续投影");
//! ```
//!
//! 宿主把每条消息按 level 写入应用日志（error→ERROR、warning→WARN、
//! info→INFO、debug→DEBUG；默认级别 INFO，debug 需 `MAESTRO_LOG_LEVEL=debug`
//! 才落盘），消息不进投影结果。message **不得携带 API Key 等凭证**——宿主
//! 不做内容审查，秘密一旦上报即会进入日志文件（见 ADR 0008）。
//!
//! # 插件配置
//!
//! 宿主在实例化之前把插件配置中 `form` 段的原始 JSON 文本就位
//! （`maestro:plugin` v2.1.0 起，见 ADR 0018），组件在 init 与
//! write-providers 之前均可经 [`get_config`] 读取：
//!
//! ```ignore
//! use maestro_plugin_sdk::get_config;
//!
//! if let Some(form) = get_config() {
//!     // form 是插件配置 form 段的原始 JSON 文本，由插件自行解析。
//! }
//! ```
//!
//! 配置缺省或无该段时返回 [`Option::None`]。宿主只透传原始文本，不解析
//! 也不审查。
//!
//! # Provider 自定义 header 与模型上限、能力
//!
//! `maestro:plugin` v2.2.0 起，`Provider` 多带一份跨协议、跨模型共享的自定义
//! HTTP header（`custom-header`，宿主按名排序），`Model` 多带声明式的 token
//! 上限（`limit`，三个子项各自可缺省，未设置即 `none`）与能力集合
//! （`capabilities`，宿主按枚举声明序排序并去重）。这些字段只作声明：宿主不
//! 据此推断、不裁剪请求、不做跨字段一致性校验，怎么投影进工具配置由插件裁定
//! （见 ADR 0019）。header 的值已完成宿主插值且**可能含凭证**，不得写进日志。
//!
//! # 原子写
//!
//! 插件把配置写进 `config_dir` 时用 [`write_atomic`]：先写同目录临时文件，
//! 成功后再改名替换——目标文件要么是旧内容，要么完全没写，绝不半截。
//!
//! ```ignore
//! use maestro_plugin_sdk::write_atomic;
//!
//! write_atomic("agent", "models.json", json.as_bytes())?;
//! ```
//!
//! # 依赖方式
//!
//! 本 crate 不发布到 crates.io，插件作者以 git 依赖引用本仓库中的 `crates/maestro-plugin-sdk`，
//! 并锁定 Maestro 的发布 tag——tag 与宿主版本一致，随 tag 固化插件与宿主的兼容组合：
//!
//! ```toml
//! [dependencies]
//! maestro-plugin-sdk = { git = "https://github.com/xezzon/agent-maestro", tag = "vX.Y.Z" }
//! ```
//!
//! 取含本 crate 的 release tag（首个此类 release 起可用）。
//!
//! # 版本对齐约定
//!
//! SDK 版本与 WIT 包 `maestro:plugin` 版本按 semver 对齐：SDK 2.x ↔ WIT 2.x，
//! 合同 breaking 变更时同步升 major；minor/patch 在 major 内各自独立。
//! `maestro:plugin` 在 major 内只加不改（只新增字段、协议与函数）；WIT 包版本
//! 升级（如 1.1.0 → 2.0.0）后宿主只注册当前版本，插件须以新版 SDK 重编译
//! （宿主恒定提供的 import 不改变这一要求）。
//! WIT 合同随本 crate 分发（`wit/maestro-plugin.wit`），构建时内嵌该文件
//! 生成绑定（见 build.rs）。

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// `maestro:plugin` v2 合同的全部 WIT 类型与 `Guest` trait。
///
/// 类型与合同 v2 一一对应：加字段/加协议 = 合同升包版本 + 插件重编译。
pub use exports::maestro::plugin::plugin::{
    CustomHeader, Endpoint, Guest, Model, ModelCapability, ModelLimit, Protocol, Provider,
};

/// 过程性日志 import 绑定（`maestro:plugin` v1.1.0 起，见 [`log`]）：
/// [`Level`] 枚举与 [`log`] 函数 re-export 到 crate 根，方便插件一行引入。
pub use maestro::plugin::logger::{Level, log};

/// 插件配置 import 绑定（`maestro:plugin` v2.1.0 起，见 [`get_config`]）：
/// `get_config` 函数 re-export 到 crate 根，方便插件一行引入。
pub use maestro::plugin::config::get_config;

/// 原子写：先写同目录临时文件，成功后再改名替换。
///
/// 插件把配置写进 `config_dir` 时最怕「写到一半」——目标文件要么是旧内容，
/// 要么完全没写，绝不半截。此前每个内置插件各写一遍 tmp + rename（连错误
/// 文案都是三份模板），这里给出唯一一份实现。
///
/// 路径相对于 `config_dir`（宿主预开放为 "/"）：`dir` 是子目录（`""` 即根），
/// `name` 是文件名。临时文件与目标文件同目录，故 rename 不跨文件系统、
/// 必为原子替换。
///
/// **`dir` 必须已存在**（本函数不建目录）：要写子目录时插件自己先
/// `create_dir_all`，例如 pi 在投影前建好 `agent/`。
///
/// ```ignore
/// use maestro_plugin_sdk::write_atomic;
///
/// write_atomic("agent", "models.json", json.as_bytes())?;
/// ```
///
/// 临时文件名逐次唯一（见下方 `create_temp_file`）：并发写同一目标时，各自
/// 写各自的临时文件，任一方都不会把另一方写到一半的内容 rename 成目标。
///
/// 失败原因带相对路径：插件作者不必自己拼错误文案。
pub fn write_atomic(dir: &str, name: &str, contents: &[u8]) -> Result<(), String> {
    let relative = Path::new(dir).join(name);
    let label = relative.to_string_lossy();
    let (tmp, mut file) =
        create_temp_file(dir, name).map_err(|e| format!("写入 {label} 临时文件失败\n原因：{e}"))?;
    // 写入与改名任一失败都清掉临时文件：不留孤儿，也不留半截的目标。
    if let Err(e) = file.write_all(contents) {
        drop(file);
        let _ = fs::remove_file(&tmp);
        return Err(format!("写入 {label} 临时文件失败\n原因：{e}"));
    }
    drop(file);
    if let Err(e) = fs::rename(&tmp, &relative) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("替换 {label} 失败\n原因：{e}"));
    }
    Ok(())
}

/// 独占创建一个临时文件（与目标同目录、名字逐次唯一）。
///
/// 名字里带纳秒时间戳与尝试序号：并发写同一目标的两次调用（同一插件被两次
/// 投影各实例化一份）不会共用同一个临时文件。`create_new` 兼作唯一性兜底——
/// 极少数撞名（同一纳秒、同一次尝试）就换序号重试。
fn create_temp_file(dir: &str, name: &str) -> io::Result<(PathBuf, fs::File)> {
    for attempt in 0..8u8 {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        let tmp = Path::new(dir).join(format!("{name}.{stamp}.{attempt}.tmp"));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(file) => return Ok((tmp, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("临时文件名重复过多"))
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::write_atomic;

    /// 用例目录的字符串形式（宿主上就是绝对路径；`write_atomic` 只做 `join`）。
    fn dir_arg(home: &tempfile::TempDir) -> String {
        home.path().to_string_lossy().into_owned()
    }

    /// 目录里的条目名（排序后比较，断言「只剩目标文件、无临时文件残留」）。
    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn writes_the_requested_file() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir(home.path().join("agent")).unwrap();
        let dir = home.path().join("agent").to_string_lossy().into_owned();

        write_atomic(&dir, "models.json", b"{\"providers\":{}}").unwrap();

        assert_eq!(
            fs::read_to_string(home.path().join("agent").join("models.json")).unwrap(),
            "{\"providers\":{}}"
        );
        assert_eq!(entries(&home.path().join("agent")), ["models.json"]);
    }

    /// 目标已存在时整体替换：旧内容更长也不会留下尾巴（不是覆盖写）。
    #[test]
    fn replaces_an_existing_target_whole() {
        let home = tempfile::tempdir().unwrap();
        let dir = dir_arg(&home);

        write_atomic(&dir, "models.json", b"{\"providers\":{\"a\":1}}").unwrap();
        write_atomic(&dir, "models.json", b"{}").unwrap();

        assert_eq!(
            fs::read_to_string(home.path().join("models.json")).unwrap(),
            "{}"
        );
        assert_eq!(entries(home.path()), ["models.json"]);
    }

    /// `dir` 不存在时报「写临时文件失败」，文案带相对路径（插件作者直接回传）。
    #[test]
    fn reports_a_missing_directory() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("missing").to_string_lossy().into_owned();

        let err = write_atomic(&dir, "models.json", b"{}").unwrap_err();

        assert!(
            err.contains("missing/models.json 临时文件失败\n原因："),
            "{err}"
        );
    }

    /// 目标被同名目录占住时 rename 必然失败：报「替换失败」，且临时文件已清掉。
    #[test]
    fn reports_a_failed_replace_and_cleans_the_temporary_file_up() {
        let home = tempfile::tempdir().unwrap();
        let dir = dir_arg(&home);
        fs::create_dir(home.path().join("models.json")).unwrap();

        let err = write_atomic(&dir, "models.json", b"{}").unwrap_err();

        assert!(err.starts_with("替换 "), "{err}");
        assert!(err.contains("models.json 失败\n原因："), "{err}");
        assert_eq!(
            entries(home.path()),
            ["models.json"],
            "失败路径不留临时文件"
        );
    }
}
