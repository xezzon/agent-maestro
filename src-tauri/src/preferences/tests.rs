//! 偏好存储的单元测试：读入归一化、原子写回与失败降级。
//!
//! 接缝是存储的公开面（构造 → 读取 → 整包更新）：输入是临时目录里预先写好的
//! 各形态偏好文件，断言读到的偏好与磁盘最终内容，不断言内部调用或私有状态。

use std::fs;

use super::*;
use crate::{errors::PreferencesError, paths::Paths};

/// 临时主目录 + 显式路径（不注入进程级全局）：偏好存储的构造函数收路径参数，
/// 测试直接指向临时目录里的偏好文件。
fn temp_paths() -> (tempfile::TempDir, Paths) {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(home.path());
    (home, paths)
}

/// 磁盘上的偏好文件内容（读回用）。
fn read_file(paths: &Paths) -> String {
    fs::read_to_string(paths.preferences_path()).unwrap()
}

/// 文件不存在：按默认值创建并写盘。
#[test]
fn missing_file_is_created_with_default_values() {
    let (_home, paths) = temp_paths();

    let store = PreferencesStore::new(paths.preferences_path());

    assert_eq!(
        store.get().unwrap(),
        Preferences {
            version: PREFERENCES_VERSION,
            theme: Theme::System
        },
        "首次启动的偏好即缺省值（跟随系统）"
    );
    assert_eq!(
        read_file(&paths),
        "{\n  \"version\": 1,\n  \"theme\": \"system\"\n}\n",
        "缺省值落盘成 version 1 / theme system"
    );
}

/// 缺字段：补缺省并写回（原文件不丢，只是被补齐）。
#[test]
fn missing_theme_field_is_filled_and_written_back() {
    let (_home, paths) = temp_paths();
    fs::create_dir_all(paths.maestro_dir()).unwrap();
    fs::write(paths.preferences_path(), r#"{"version":1}"#).unwrap();

    let store = PreferencesStore::new(paths.preferences_path());

    assert_eq!(store.get().unwrap().theme, Theme::System, "缺 theme 即缺省");
    assert_eq!(
        read_file(&paths),
        "{\n  \"version\": 1,\n  \"theme\": \"system\"\n}\n",
        "缺字段补齐后写回"
    );
}

/// 缺 version：同样按缺省补齐（缺字段一律补，不当版本不受支持处理）。
#[test]
fn missing_version_field_is_filled_and_written_back() {
    let (_home, paths) = temp_paths();
    fs::create_dir_all(paths.maestro_dir()).unwrap();
    fs::write(paths.preferences_path(), r#"{"theme":"dark"}"#).unwrap();

    let store = PreferencesStore::new(paths.preferences_path());

    assert_eq!(store.get().unwrap().version, PREFERENCES_VERSION);
    assert_eq!(store.get().unwrap().theme, Theme::Dark, "已有字段不受影响");
    assert!(
        read_file(&paths).contains(r#""version": 1"#),
        "缺 version 补齐后写回：{}",
        read_file(&paths)
    );
}

/// 非法主题值：兜底为跟随系统并落盘修正。
#[test]
fn invalid_theme_value_falls_back_to_system_and_is_corrected() {
    let (_home, paths) = temp_paths();
    fs::create_dir_all(paths.maestro_dir()).unwrap();
    fs::write(paths.preferences_path(), r#"{"version":1,"theme":"neon"}"#).unwrap();

    let store = PreferencesStore::new(paths.preferences_path());

    assert_eq!(
        store.get().unwrap().theme,
        Theme::System,
        "非法值兜底为跟随系统"
    );
    assert_eq!(
        read_file(&paths),
        "{\n  \"version\": 1,\n  \"theme\": \"system\"\n}\n",
        "兜底值落盘修正，下次打开不再撞上同一个问题"
    );
}

/// 合法主题值：原样读入，不被改写。
#[test]
fn valid_theme_values_are_read_verbatim() {
    let (_home, paths) = temp_paths();
    fs::create_dir_all(paths.maestro_dir()).unwrap();
    fs::write(paths.preferences_path(), r#"{"version":1,"theme":"dark"}"#).unwrap();

    let store = PreferencesStore::new(paths.preferences_path());

    assert_eq!(store.get().unwrap().theme, Theme::Dark);
    assert_eq!(
        read_file(&paths),
        r#"{"version":1,"theme":"dark"}"#,
        "读得出来就不改写用户的文件"
    );
}

/// JSON 语法坏：原文件留档为 `preferences.json.bad`，再写默认值。
#[test]
fn corrupt_file_is_archived_and_defaults_are_written() {
    let (_home, paths) = temp_paths();
    fs::create_dir_all(paths.maestro_dir()).unwrap();
    fs::write(paths.preferences_path(), "不是 JSON {{{").unwrap();

    let store = PreferencesStore::new(paths.preferences_path());

    assert_eq!(
        store.get().unwrap().theme,
        Theme::System,
        "坏文件按缺省值启动"
    );
    let backup = paths
        .preferences_path()
        .with_file_name("preferences.json.bad");
    assert_eq!(
        fs::read_to_string(&backup).unwrap(),
        "不是 JSON {{{",
        "原文件留档，不静默销毁用户手写的内容"
    );
    assert_eq!(
        read_file(&paths),
        "{\n  \"version\": 1,\n  \"theme\": \"system\"\n}\n",
        "留档后写回默认值"
    );
}

/// 版本不受支持：按默认值运行，且不读写该文件。
#[test]
fn unsupported_version_runs_with_defaults_and_leaves_the_file_untouched() {
    let (_home, paths) = temp_paths();
    fs::create_dir_all(paths.maestro_dir()).unwrap();
    let original = r#"{"version":99,"theme":"dark"}"#;
    fs::write(paths.preferences_path(), original).unwrap();

    let store = PreferencesStore::new(paths.preferences_path());

    assert_eq!(
        store.get().unwrap(),
        Preferences {
            version: PREFERENCES_VERSION,
            theme: Theme::System
        },
        "版本不受支持时按默认值运行"
    );
    assert_eq!(
        read_file(&paths),
        original,
        "版本不受支持的文件不改写（可能来自更新的版本）"
    );
}

/// 版本写成 `null`：与字段缺失同义，按缺省补齐并写回，不当版本不受支持处理。
#[test]
fn null_version_is_filled_like_a_missing_field() {
    let (_home, paths) = temp_paths();
    fs::create_dir_all(paths.maestro_dir()).unwrap();
    fs::write(
        paths.preferences_path(),
        r#"{"version":null,"theme":"dark"}"#,
    )
    .unwrap();

    let store = PreferencesStore::new(paths.preferences_path());

    assert_eq!(store.get().unwrap().version, PREFERENCES_VERSION);
    assert_eq!(store.get().unwrap().theme, Theme::Dark, "已有字段不受影响");
    assert_eq!(
        read_file(&paths),
        "{\n  \"version\": 1,\n  \"theme\": \"dark\"\n}\n",
        "null 补齐为当前版本后写回，该文件仍归当前版本管"
    );

    // 归当前版本管：后续更新不被拒。
    let updated = store
        .update(Preferences {
            version: PREFERENCES_VERSION,
            theme: Theme::Light,
        })
        .unwrap();
    assert_eq!(updated.theme, Theme::Light);
}

/// 版本写成非整数（`1.0`）：不是「更新版本写出」的文件，但形状已坏——按默认值
/// 运行且不碰该文件，随后的更新同样被拒。
#[test]
fn non_integer_version_is_left_untouched_with_a_garbage_reason() {
    let (_home, paths) = temp_paths();
    fs::create_dir_all(paths.maestro_dir()).unwrap();
    let original = r#"{"version":1.0,"theme":"dark"}"#;
    fs::write(paths.preferences_path(), original).unwrap();

    let store = PreferencesStore::new(paths.preferences_path());

    assert_eq!(
        store.get().unwrap(),
        Preferences {
            version: PREFERENCES_VERSION,
            theme: Theme::System
        },
        "形状坏了按默认值运行"
    );
    assert_eq!(read_file(&paths), original, "非整数版本的文件不读写");

    let err = store
        .update(Preferences {
            version: PREFERENCES_VERSION,
            theme: Theme::Light,
        })
        .unwrap_err();
    match err {
        PreferencesError::UnsupportedVersion { found, integer, .. } => {
            assert_eq!(found, "1.0");
            assert!(!integer, "非整数取值不被当成更新版本写出的文件");
        }
        other => panic!("{other:?}"),
    }
}

/// 版本不受支持：随后切主题也不得覆盖那份文件。
///
/// 加载期的「不读写」只挡住了启动那一刻——若存储层不记住这件事，用户第一次
/// 切主题就会用 version 1 覆盖掉新版写下的偏好，正是用户故事 13 要防的降级
/// 销毁。
#[test]
fn update_does_not_overwrite_a_file_from_a_newer_version() {
    let (_home, paths) = temp_paths();
    fs::create_dir_all(paths.maestro_dir()).unwrap();
    let original = r#"{"version":99,"theme":"dark"}"#;
    fs::write(paths.preferences_path(), original).unwrap();
    let store = PreferencesStore::new(paths.preferences_path());

    let err = store
        .update(Preferences {
            version: PREFERENCES_VERSION,
            theme: Theme::Light,
        })
        .unwrap_err();

    assert!(
        matches!(err, PreferencesError::UnsupportedVersion { .. }),
        "{err:?}"
    );
    assert_eq!(
        read_file(&paths),
        original,
        "被拒的更新不落盘：新版写下的文件不被旧版覆盖"
    );
    assert_eq!(
        store.get().unwrap().theme,
        Theme::System,
        "被拒的更新不改内存状态"
    );
}

/// 整包替换：写盘后重新打开读回一致（原子写 round-trip）。
#[test]
fn update_replaces_the_whole_document_and_round_trips() {
    let (_home, paths) = temp_paths();
    let store = PreferencesStore::new(paths.preferences_path());

    let updated = store
        .update(Preferences {
            version: PREFERENCES_VERSION,
            theme: Theme::Dark,
        })
        .unwrap();

    assert_eq!(updated.theme, Theme::Dark);
    assert_eq!(
        read_file(&paths),
        "{\n  \"version\": 1,\n  \"theme\": \"dark\"\n}\n"
    );
    let reopened = PreferencesStore::new(paths.preferences_path());
    assert_eq!(
        reopened.get().unwrap().theme,
        Theme::Dark,
        "落盘后重新打开读回一致"
    );
}

/// 写入前强制版本号：载荷带的版本不生效，落盘的一律是当前版本。
#[test]
fn update_forces_the_current_version() {
    let (_home, paths) = temp_paths();
    let store = PreferencesStore::new(paths.preferences_path());

    let updated = store
        .update(Preferences {
            version: 99,
            theme: Theme::Light,
        })
        .unwrap();

    assert_eq!(updated.version, PREFERENCES_VERSION);
    assert!(
        read_file(&paths).contains(r#""version": 1"#),
        "落盘的是当前版本：{}",
        read_file(&paths)
    );
}

/// 读不出来（目标路径不是文件）：按兜底值启动，不阻断。
#[test]
fn unreadable_file_falls_back_to_defaults() {
    let (_home, paths) = temp_paths();
    // 目标路径是目录：读不出来（也写不进去）。
    fs::create_dir_all(paths.preferences_path()).unwrap();

    let store = PreferencesStore::new(paths.preferences_path());

    assert_eq!(
        store.get().unwrap().theme,
        Theme::System,
        "读不出来也照常启动"
    );
}

/// 写盘失败：报错，且内存状态保持原值（界面据此回滚）。
#[test]
fn failed_write_does_not_change_the_in_memory_value() {
    let (_home, paths) = temp_paths();
    fs::create_dir_all(paths.preferences_path()).unwrap();
    let store = PreferencesStore::new(paths.preferences_path());

    let err = store
        .update(Preferences {
            version: PREFERENCES_VERSION,
            theme: Theme::Dark,
        })
        .unwrap_err();

    assert!(matches!(err, PreferencesError::Io { .. }), "{err:?}");
    assert_eq!(
        store.get().unwrap().theme,
        Theme::System,
        "写失败的更新不改内存状态"
    );
}

/// 启动时的写回失败：只记 WARN，内存继续用兜底值，原文件原样保留。
///
/// 写失败用「目录不可写」制造。root 无视目录权限，故先探一次：探得到写权限
/// 说明本用例造不出写失败（其余用例仍覆盖同一条兜底路径），直接跳过。
#[cfg(unix)]
#[test]
fn write_back_failure_does_not_block_startup() {
    use std::os::unix::fs::PermissionsExt;

    let (_home, paths) = temp_paths();
    let dir = paths.maestro_dir();
    fs::create_dir_all(&dir).unwrap();
    let original = r#"{"version":1}"#;
    fs::write(paths.preferences_path(), original).unwrap();

    let mode = fs::metadata(&dir).unwrap().permissions().mode();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
    let probe = dir.join(".write-probe");
    let writable = fs::write(&probe, b"").is_ok();
    let _ = fs::remove_file(&probe);
    if writable {
        fs::set_permissions(&dir, fs::Permissions::from_mode(mode)).unwrap();
        return;
    }

    let store = PreferencesStore::new(paths.preferences_path());

    fs::set_permissions(&dir, fs::Permissions::from_mode(mode)).unwrap();
    assert_eq!(
        store.get().unwrap().theme,
        Theme::System,
        "写不回去也照常以兜底值启动"
    );
    assert_eq!(read_file(&paths), original, "写不回去时原文件原样保留");
}
