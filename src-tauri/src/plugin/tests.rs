//! 插件服务的单元测试：安装、启停、移除、重载、投影与插件配置读写。

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use super::{
    PLACED_CONFIG, PLACED_MANIFEST, PLACED_WASM, PlacedPlugin, PluginApplyReport, PluginConfig,
    PluginEntry, PluginState, PluginView, builtin,
    manifest::is_https_url,
    projection::project_files,
    testutil::{
        LockProbeFetcher, StubFetcher, fetcher_service, manifest_json, snapshot, store_at,
        stub_service, temp_home, test_service,
    },
};
use crate::{paths::MaestroPaths, provider::Provider};

const MANIFEST_URL: &str = "https://example.com/releases/download/v1/manifest.json";
const WASM_URL: &str = "https://example.com/releases/download/v1/plugin.wasm";

/// 按 id 取配置条目：内置插件已不止一个，固定的下标不再可用。
fn plugin_entry<'a>(plugins: &'a [PluginEntry], id: &str) -> &'a PluginEntry {
    plugins
        .iter()
        .find(|entry| entry.id == id)
        .unwrap_or_else(|| panic!("配置条目「{id}」不存在"))
}

fn plugin_view<'a>(views: &'a [PluginView], id: &str) -> &'a PluginView {
    views
        .iter()
        .find(|view| view.id == id)
        .unwrap_or_else(|| panic!("插件「{id}」不在列表中"))
}

fn plugin_report<'a>(reports: &'a [PluginApplyReport], id: &str) -> &'a PluginApplyReport {
    reports
        .iter()
        .find(|report| report.id == id)
        .unwrap_or_else(|| panic!("插件「{id}」没有投影报告"))
}

/// https 来源的替身：manifest 的 entry 指向 Release 资产，产物用内置 pi wasm 充当
/// （内置 pi 即一份符合 WIT 合同的真实组件）。
fn https_stub(id: &str) -> StubFetcher {
    let fetcher = StubFetcher::new();
    fetcher.serve(
        MANIFEST_URL,
        manifest_json(id, &format!("$HOME/.{id}"), WASM_URL),
    );
    fetcher.serve(WASM_URL, builtin::PI_WASM.to_vec());
    fetcher
}

/// 本机插件项目目录的测试替身：写入 manifest.json 与 entry 指向的产物。
fn source_dir(id: &str, entry: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("manifest.json");
    fs::write(&manifest, manifest_json(id, &format!("$HOME/.{id}"), entry)).unwrap();
    if !is_https_url(entry) {
        let artifact = dir.path().join(entry);
        fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        fs::write(&artifact, builtin::PI_WASM).unwrap();
    }
    (dir, manifest)
}

#[test]
fn add_https_plugin_writes_entry_places_files_and_loads() {
    let home = temp_home();
    let store = store_at(home.path());
    let service = stub_service(home.path(), https_stub("pi2"));
    let maestro_paths = MaestroPaths::get();

    service.add_plugin(&store, MANIFEST_URL).unwrap();

    // 条目：id 在安装成功后落盘，是来源到落位目录的唯一映射（见 ADR 0006）。
    let plugins = snapshot(&store).plugins;
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].source, MANIFEST_URL);
    assert_eq!(plugins[0].id, "pi2");
    assert!(plugins[0].enabled);

    // 落位目录：manifest 原样保留上游内容（entry 仍是回源地址）+ 固定名 wasm。
    let dir = maestro_paths.plugin_dir("pi2");
    assert_eq!(
        fs::read_to_string(dir.join(PLACED_MANIFEST)).unwrap(),
        manifest_json("pi2", "$HOME/.pi2", WASM_URL)
    );
    assert_eq!(
        fs::read(dir.join(PLACED_WASM)).unwrap(),
        builtin::PI_WASM.to_vec()
    );

    let views = service.list(&store).unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].error, None, "{:?}", views[0].error);
    assert_eq!(views[0].id, "pi2");
    assert!(!views[0].builtin, "https 来源可重新加载、可移除");
    let config_dir = views[0].config_dir.as_deref().unwrap();
    assert!(
        Path::new(config_dir).is_absolute() && config_dir.ends_with(".pi2"),
        "config_dir 已解析为宿主内的绝对路径：{config_dir}"
    );
}

#[test]
fn add_file_plugin_reads_manifest_and_entry_from_the_source_dir() {
    let home = temp_home();
    let store = store_at(home.path());
    let maestro_paths = MaestroPaths::get();
    let service = test_service(home.path());
    let entry = "target/wasm32-wasip2/release/maestro_plugin_pi.wasm";
    let (source, manifest) = source_dir("pi2", entry);
    let source_id = manifest.display().to_string();

    service.add_plugin(&store, &source_id).unwrap();

    assert_eq!(snapshot(&store).plugins[0].id, "pi2");
    let dir = maestro_paths.plugin_dir("pi2");
    assert_eq!(
        fs::read_to_string(dir.join(PLACED_MANIFEST)).unwrap(),
        fs::read_to_string(&manifest).unwrap(),
        "落位 manifest 为来源原文"
    );
    assert_eq!(
        fs::read(dir.join(PLACED_WASM)).unwrap(),
        builtin::PI_WASM.to_vec()
    );
    assert_eq!(service.list(&store).unwrap()[0].error, None);
    assert!(
        source.path().join(entry).exists(),
        "只删落位副本，不动用户的插件项目目录"
    );
}

#[test]
fn add_plugin_rejects_unrecognized_sources_before_any_io() {
    let home = temp_home();
    let store = store_at(home.path());
    let service = test_service(home.path());

    for source in [
        "http://example.com/manifest.json",
        "file:///tmp/manifest.json",
        "git://example.com/x.git",
        "plugins/pi/manifest.json",
    ] {
        let err = service.add_plugin(&store, source).unwrap_err();
        assert!(
            err.contains("无法识别的插件来源"),
            "{source} 应被拒绝：{err}"
        );
        assert!(
            !err.contains("未预置的 URL"),
            "来源形态的判定必须在发起下载之前：{err}"
        );
    }
    assert!(
        snapshot(&store).plugins.is_empty(),
        "被拒绝的来源不得留下条目"
    );
}

#[test]
fn add_plugin_rejects_duplicate_source_before_downloading() {
    let home = temp_home();
    let store = store_at(home.path());
    let fetcher = Arc::new(https_stub("pi2"));
    let service = fetcher_service(home.path(), fetcher.clone());
    service.add_plugin(&store, MANIFEST_URL).unwrap();
    // 第二次添加：下载若发生必然失败，据此确认重复检查先于下载。
    fetcher.fail(MANIFEST_URL, "网络不可达");

    let err = service.add_plugin(&store, MANIFEST_URL).unwrap_err();

    assert!(err.contains("已存在同一来源"), "{err}");
    assert_eq!(snapshot(&store).plugins.len(), 1);
}

#[test]
fn add_plugin_rejects_id_conflict_without_placing() {
    let home = temp_home();
    MaestroPaths::init(home.path());
    let maestro_paths = MaestroPaths::get();
    fs::create_dir_all(maestro_paths.maestro_dir()).unwrap();
    fs::write(
        maestro_paths.config_path(),
        format!(
            r#"{{"version":1,"providers":{{}},"plugins":[{{"source":"{}","enabled":true,"id":"pi"}}]}}"#,
            builtin::BUILTIN_PI_SOURCE
        ),
    )
    .unwrap();
    let store = store_at(home.path());
    let service = stub_service(home.path(), https_stub("pi"));

    let err = service.add_plugin(&store, MANIFEST_URL).unwrap_err();

    assert!(err.contains("已被来源 builtin:pi 占用"), "{err}");
    assert_eq!(snapshot(&store).plugins.len(), 1, "冲突不得写条目");
    assert!(!maestro_paths.plugin_dir("pi").exists(), "冲突不得落位");
}

#[test]
fn add_plugin_writes_nothing_when_download_fails() {
    let home = temp_home();
    let store = store_at(home.path());
    let maestro_paths = MaestroPaths::get();
    let fetcher = StubFetcher::new();
    fetcher.fail(MANIFEST_URL, "网络不可达");
    let service = stub_service(home.path(), fetcher);

    let err = service.add_plugin(&store, MANIFEST_URL).unwrap_err();

    assert!(
        err.contains("下载 manifest 失败") && err.contains("网络不可达"),
        "{err}"
    );
    assert!(snapshot(&store).plugins.is_empty(), "安装失败不得写条目");
    assert!(!maestro_paths.plugins_dir().exists(), "安装失败不得落位");
}

#[test]
fn add_plugin_rejects_wasm_that_is_not_a_component() {
    let home = temp_home();
    let store = store_at(home.path());
    let maestro_paths = MaestroPaths::get();
    let fetcher = StubFetcher::new();
    fetcher.serve(MANIFEST_URL, manifest_json("pi2", "$HOME/.pi2", WASM_URL));
    fetcher.serve(WASM_URL, b"not a wasm component".to_vec());
    let service = stub_service(home.path(), fetcher);

    let err = service.add_plugin(&store, MANIFEST_URL).unwrap_err();

    assert!(err.contains("不是有效的 WASM 组件"), "{err}");
    assert!(snapshot(&store).plugins.is_empty());
    assert!(
        !maestro_paths.plugin_dir("pi2").exists(),
        "校验失败不得落位"
    );
}

#[test]
fn add_plugin_rejects_config_dir_escaping_its_base() {
    let home = temp_home();
    let store = store_at(home.path());
    let maestro_paths = MaestroPaths::get();
    let fetcher = StubFetcher::new();
    fetcher.serve(
        MANIFEST_URL,
        // `~` 与 `$HOME` 同义：从基准目录上跳同样被拒。
        manifest_json("pi2", "~/.pi2/../../etc", WASM_URL),
    );
    fetcher.serve(WASM_URL, builtin::PI_WASM.to_vec());
    let service = stub_service(home.path(), fetcher);

    let err = service.add_plugin(&store, MANIFEST_URL).unwrap_err();

    assert!(err.contains("config_dir"), "{err}");
    assert!(snapshot(&store).plugins.is_empty());
    assert!(!maestro_paths.plugin_dir("pi2").exists());
}

/// 下载可能持续数秒：全程不得持有配置存储锁，否则会连锁阻塞所有 Provider / 插件命令。
#[test]
fn add_plugin_does_not_hold_the_store_lock_while_downloading() {
    let home = temp_home();
    let store = Arc::new(store_at(home.path()));
    let service = fetcher_service(
        home.path(),
        Arc::new(LockProbeFetcher::new(Arc::clone(&store), https_stub("pi2"))),
    );

    service.add_plugin(&store, MANIFEST_URL).unwrap();

    assert!(service.list(&store).unwrap()[0].error.is_none());
}

/// 同 id、不同来源的并发 install：install_lock 串行化整个流程，
/// 恰好一个成功、另一个失败，赢家的落位目录必须保留。
/// 修复前的 race:两个线程都通过预检、各自 save 到同一 plugin_dir(id)、
/// 后写入 store.add_plugin 的线程报错后调用 uninstall 误删赢家的文件。
#[test]
fn concurrent_add_plugin_with_same_id_serializes_and_preserves_winner() {
    use std::thread;

    let home = temp_home();
    let home_path = home.path().to_path_buf();

    let source_a = "https://example.com/a/manifest.json";
    let source_b = "https://example.com/b/manifest.json";
    let wasm_a = "https://example.com/a/plugin.wasm";
    let wasm_b = "https://example.com/b/plugin.wasm";
    let fetcher = StubFetcher::new();
    fetcher.serve(source_a, manifest_json("pi", "$HOME/.pi", wasm_a));
    fetcher.serve(source_b, manifest_json("pi", "$HOME/.pi", wasm_b));
    fetcher.serve(wasm_a, builtin::PI_WASM.to_vec());
    fetcher.serve(wasm_b, builtin::PI_WASM.to_vec());

    let store = Arc::new(store_at(home.path()));
    let service = Arc::new(stub_service(home.path(), fetcher));

    let handles = vec![
        {
            let store = Arc::clone(&store);
            let service = Arc::clone(&service);
            let home = home_path.clone();
            thread::spawn(move || {
                // 子线程 thread_local 为空，需重新设置全局目录。
                crate::plugin::testutil::setup_test_dirs(&home);
                service.add_plugin(&store, source_a)
            })
        },
        {
            let store = Arc::clone(&store);
            let service = Arc::clone(&service);
            let home = home_path.clone();
            thread::spawn(move || {
                crate::plugin::testutil::setup_test_dirs(&home);
                service.add_plugin(&store, source_b)
            })
        },
    ];
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    let successes = results.iter().filter(|r| r.is_ok()).count();
    let failures = results.iter().filter(|r| r.is_err()).count();
    assert_eq!(successes, 1, "恰好一个 install 成功：{results:?}");
    assert_eq!(failures, 1, "另一个 install 失败：{results:?}");

    // 配置文件中恰好一条记录，赢家独享。
    let plugins = snapshot(&store).plugins;
    assert_eq!(plugins.len(), 1, "只有一个 id 落盘");
    assert_eq!(plugins[0].id, "pi", "赢家的 id 落盘");

    // 赢家的落位目录与产物必须保留——这是修复的核心不变量。
    let placed_dir = MaestroPaths::get().plugin_dir("pi");
    assert!(
        placed_dir.join(PLACED_MANIFEST).exists(),
        "赢家的 manifest.json 必须保留"
    );
    assert!(
        placed_dir.join(PLACED_WASM).exists(),
        "赢家的 plugin.wasm 必须保留"
    );
}

#[test]
fn set_enabled_disables_and_re_enables_the_plugin() {
    let home = temp_home();
    let store = store_at(home.path());
    let service = stub_service(home.path(), https_stub("pi2"));
    service.add_plugin(&store, MANIFEST_URL).unwrap();

    service.set_enabled(&store, MANIFEST_URL, false).unwrap();
    assert!(!snapshot(&store).plugins[0].enabled);
    let view = &service.list(&store).unwrap()[0];
    assert!(!view.enabled);
    assert_eq!(view.error, None);
    assert_eq!(view.id, "pi2");
    assert!(
        view.config_dir.as_deref().unwrap().ends_with(".pi2"),
        "禁用保留 manifest 供展示：{:?}",
        view.config_dir
    );

    service.set_enabled(&store, MANIFEST_URL, true).unwrap();
    assert!(snapshot(&store).plugins[0].enabled);
    let view = &service.list(&store).unwrap()[0];
    assert!(view.enabled);
    assert_eq!(view.error, None);
    assert_eq!(view.id, "pi2");
}

#[test]
fn set_enabled_enable_fails_without_touching_config_when_placed_files_are_missing() {
    let home = temp_home();
    let store = store_at(home.path());
    let maestro_paths = MaestroPaths::get();
    let service = stub_service(home.path(), https_stub("pi2"));
    service.add_plugin(&store, MANIFEST_URL).unwrap();
    service.set_enabled(&store, MANIFEST_URL, false).unwrap();
    fs::remove_dir_all(maestro_paths.plugin_dir("pi2")).unwrap();

    let err = service.set_enabled(&store, MANIFEST_URL, true).unwrap_err();

    assert!(err.contains("读取"), "{err}");
    assert!(
        !snapshot(&store).plugins[0].enabled,
        "装载失败不得改配置条目"
    );
    assert!(!service.list(&store).unwrap()[0].enabled);
}

#[test]
fn set_enabled_unknown_source_is_rejected() {
    let home = temp_home();
    let store = store_at(home.path());
    let service = test_service(home.path());

    let err = service
        .set_enabled(&store, "builtin:ghost", true)
        .unwrap_err();

    assert!(err.contains("插件条目不存在"), "{err}");
}

#[test]
fn remove_plugin_unloads_memory_deletes_entry_then_dir() {
    let home = temp_home();
    let store = store_at(home.path());
    let maestro_paths = MaestroPaths::get();
    let service = stub_service(home.path(), https_stub("pi2"));
    service.add_plugin(&store, MANIFEST_URL).unwrap();
    // 插件配置随插件存在（ADR 0018）：移除连落位目录带 config.json 一并删除。
    store
        .write()
        .unwrap()
        .set_variables(BTreeMap::from([(
            "HOST".to_owned(),
            "api.example.com".to_owned(),
        )]))
        .unwrap();
    service
        .set_config(
            &store,
            MANIFEST_URL,
            &PluginConfig {
                variables: BTreeMap::from([("HOST".to_owned(), "gateway.local".to_owned())]),
                form: None,
            },
        )
        .unwrap();
    assert!(
        maestro_paths.plugin_dir("pi2").join(PLACED_CONFIG).exists(),
        "前置：插件配置已落盘"
    );

    service.remove_plugin(&store, MANIFEST_URL).unwrap();

    assert!(snapshot(&store).plugins.is_empty(), "配置条目已删除");
    assert!(!maestro_paths.plugin_dir("pi2").exists(), "落位目录已删除");
    assert!(
        !maestro_paths.plugin_dir("pi2").join(PLACED_CONFIG).exists(),
        "插件配置随落位目录一并消失"
    );
    assert!(
        service.entries.read().unwrap().is_empty(),
        "内存注册表已卸载"
    );
    assert!(service.list(&store).unwrap().is_empty());
}

#[test]
fn remove_plugin_is_idempotent_when_the_entry_is_already_gone() {
    let home = temp_home();
    let store = store_at(home.path());
    let service = test_service(home.path());

    service
        .remove_plugin(&store, MANIFEST_URL)
        .expect("条目不存在同样成功");
    service
        .remove_plugin(&store, MANIFEST_URL)
        .expect("移除两次同样成功");
}

#[test]
fn remove_plugin_rejects_builtin_source() {
    let home = temp_home();
    let store = store_at(home.path());
    let service = test_service(home.path());

    let err = service
        .remove_plugin(&store, builtin::BUILTIN_PI_SOURCE)
        .unwrap_err();

    assert!(err.contains("内置插件不可移除"), "{err}");
}

#[test]
fn remove_plugin_keeps_the_user_project_dir_for_file_sources() {
    let home = temp_home();
    let store = store_at(home.path());
    let maestro_paths = MaestroPaths::get();
    let service = test_service(home.path());
    let entry = "target/wasm32-wasip2/release/maestro_plugin_pi.wasm";
    let (source, manifest) = source_dir("pi2", entry);
    let source_id = manifest.display().to_string();
    service.add_plugin(&store, &source_id).unwrap();

    service.remove_plugin(&store, &source_id).unwrap();

    assert!(snapshot(&store).plugins.is_empty());
    assert!(!maestro_paths.plugin_dir("pi2").exists());
    assert!(
        source.path().join(entry).exists(),
        "只删落位副本，不动用户的插件项目目录"
    );
}

#[test]
fn reload_plugin_refetches_and_replaces_the_placed_version() {
    let home = temp_home();
    let store = store_at(home.path());
    let maestro_paths = MaestroPaths::get();
    let fetcher = Arc::new(https_stub("pi2"));
    let service = fetcher_service(home.path(), fetcher.clone());
    service.add_plugin(&store, MANIFEST_URL).unwrap();

    // 上游改版：同 id、新的 config_dir。
    fetcher.serve(MANIFEST_URL, manifest_json("pi2", "$HOME/.pi3", WASM_URL));

    service.reload_plugin(&store, MANIFEST_URL).unwrap();

    let dir = maestro_paths.plugin_dir("pi2");
    assert_eq!(
        fs::read_to_string(dir.join(PLACED_MANIFEST)).unwrap(),
        manifest_json("pi2", "$HOME/.pi3", WASM_URL),
        "落位目录已替换为新版本"
    );
    let view = &service.list(&store).unwrap()[0];
    assert_eq!(view.error, None);
    assert_eq!(view.id, "pi2");
    assert!(
        view.config_dir.as_deref().unwrap().ends_with(".pi3"),
        "注册表同步新 manifest：{:?}",
        view.config_dir
    );
}

/// 逐文件覆盖（见 ADR 0018）：重新加载只替换 manifest.json 与 plugin.wasm
/// 两个宿主产物，落位目录中的其余文件（如插件配置）保持原样。
#[test]
fn reload_plugin_keeps_unrelated_files_in_the_placed_dir() {
    let home = temp_home();
    let store = store_at(home.path());
    let maestro_paths = MaestroPaths::get();
    let fetcher = Arc::new(https_stub("pi2"));
    let service = fetcher_service(home.path(), fetcher.clone());
    service.add_plugin(&store, MANIFEST_URL).unwrap();

    let user_file = maestro_paths.plugin_dir("pi2").join("config.json");
    fs::write(&user_file, r#"{"variables":{}}"#).unwrap();

    fetcher.serve(MANIFEST_URL, manifest_json("pi2", "$HOME/.pi3", WASM_URL));
    service.reload_plugin(&store, MANIFEST_URL).unwrap();

    assert_eq!(
        fs::read_to_string(&user_file).unwrap(),
        r#"{"variables":{}}"#,
        "落位目录中的非宿主产物文件不被重载冲掉"
    );
    assert_eq!(
        fs::read_to_string(maestro_paths.plugin_dir("pi2").join(PLACED_MANIFEST)).unwrap(),
        manifest_json("pi2", "$HOME/.pi3", WASM_URL),
        "宿主产物照常覆盖为新版本"
    );
}

/// 「重载失败旧版本保持可用」的保证已取消（见 ADR 0018）：先 manifest、后 wasm
/// 的逐文件覆盖在写 wasm 失败时停在「新 manifest + 旧 wasm」的混合态，
/// 错误直接返回、注册表保持旧状态，恢复手段是从来源重新加载。
#[test]
fn reload_plugin_write_failure_leaves_a_mixed_placed_state() {
    let home = temp_home();
    let store = store_at(home.path());
    let maestro_paths = MaestroPaths::get();
    let fetcher = Arc::new(https_stub("pi2"));
    let service = fetcher_service(home.path(), fetcher.clone());
    service.add_plugin(&store, MANIFEST_URL).unwrap();

    // 上游改版后，把落位的 plugin.wasm 换成同名目录：覆盖 wasm 必然失败。
    fetcher.serve(MANIFEST_URL, manifest_json("pi2", "$HOME/.pi3", WASM_URL));
    let placed_wasm = maestro_paths.plugin_dir("pi2").join(PLACED_WASM);
    fs::remove_file(&placed_wasm).unwrap();
    fs::create_dir(&placed_wasm).unwrap();

    let err = service.reload_plugin(&store, MANIFEST_URL).unwrap_err();

    assert!(err.contains(PLACED_WASM), "{err}");
    assert_eq!(
        fs::read_to_string(maestro_paths.plugin_dir("pi2").join(PLACED_MANIFEST)).unwrap(),
        manifest_json("pi2", "$HOME/.pi3", WASM_URL),
        "manifest 已覆盖为新版本，落位处于混合态"
    );
    assert!(placed_wasm.is_dir(), "覆盖失败的 wasm 保持原样");
    let view = &service.list(&store).unwrap()[0];
    assert_eq!(view.error, None);
    assert_eq!(view.id, "pi2");
    assert!(
        view.config_dir.as_deref().unwrap().ends_with(".pi2"),
        "注册表保持旧状态：{:?}",
        view.config_dir
    );
}

#[test]
fn reload_plugin_rejects_upstream_id_change_and_keeps_old_state() {
    let home = temp_home();
    let store = store_at(home.path());
    let maestro_paths = MaestroPaths::get();
    let fetcher = Arc::new(https_stub("pi2"));
    let service = fetcher_service(home.path(), fetcher.clone());
    service.add_plugin(&store, MANIFEST_URL).unwrap();

    // 上游把 id 改成了 pi3：落位目录由条目 id 定位，id 漂移会架空映射（见 ADR 0006）。
    fetcher.serve(MANIFEST_URL, manifest_json("pi3", "$HOME/.pi2", WASM_URL));

    let err = service.reload_plugin(&store, MANIFEST_URL).unwrap_err();

    assert!(
        err.contains("pi2") && err.contains("pi3") && err.contains("拒绝重新加载"),
        "{err}"
    );
    assert_eq!(
        snapshot(&store).plugins[0].id,
        "pi2",
        "拒绝时不得改配置条目"
    );
    let dir = maestro_paths.plugin_dir("pi2");
    assert_eq!(
        fs::read_to_string(dir.join(PLACED_MANIFEST)).unwrap(),
        manifest_json("pi2", "$HOME/.pi2", WASM_URL),
        "拒绝时旧版本保持可用"
    );
    assert_eq!(service.list(&store).unwrap()[0].id, "pi2");
    assert!(
        !maestro_paths.plugin_dir("pi3").exists(),
        "拒绝时不得落位新目录"
    );
}

#[test]
fn reload_plugin_keeps_the_disabled_state_with_a_fresh_manifest() {
    let home = temp_home();
    let store = store_at(home.path());
    let maestro_paths = MaestroPaths::get();
    let fetcher = Arc::new(https_stub("pi2"));
    let service = fetcher_service(home.path(), fetcher.clone());
    service.add_plugin(&store, MANIFEST_URL).unwrap();
    service.set_enabled(&store, MANIFEST_URL, false).unwrap();

    fetcher.serve(MANIFEST_URL, manifest_json("pi2", "$HOME/.pi3", WASM_URL));
    service.reload_plugin(&store, MANIFEST_URL).unwrap();

    assert!(
        !snapshot(&store).plugins[0].enabled,
        "重新加载不改条目的 enabled"
    );
    let dir = maestro_paths.plugin_dir("pi2");
    assert_eq!(
        fs::read_to_string(dir.join(PLACED_MANIFEST)).unwrap(),
        manifest_json("pi2", "$HOME/.pi3", WASM_URL)
    );
    let view = &service.list(&store).unwrap()[0];
    assert!(!view.enabled);
    assert_eq!(view.error, None);
    assert_eq!(view.id, "pi2");
    assert!(
        view.config_dir.as_deref().unwrap().ends_with(".pi3"),
        "禁用态仍以新 manifest 供展示：{:?}",
        view.config_dir
    );
}

#[test]
fn reload_plugin_rejects_unknown_sources() {
    let home = temp_home();
    let store = store_at(home.path());
    let service = test_service(home.path());

    let err = service.reload_plugin(&store, MANIFEST_URL).unwrap_err();
    assert!(err.contains("插件条目不存在"), "{err}");
}

/// 内置插件的「重新加载」按内嵌字节重建落位副本：损坏的宿主副本可被恢复，
/// 配置条目（`enabled` 等）保持不变。
#[test]
fn reload_builtin_plugin_restores_a_corrupted_placed_copy() {
    let home = temp_home();
    let store = store_at(home.path());
    let maestro_paths = MaestroPaths::get();
    let service = test_service(home.path());

    service.startup(&store).unwrap();
    assert_eq!(
        snapshot(&store).plugins.len(),
        builtin::BUILTINS.len(),
        "启动补装全部内置插件"
    );

    // 故意损坏宿主落位的 wasm：重新加载必须按内嵌字节还原。
    let placed_wasm = maestro_paths.plugin_dir("pi").join(PLACED_WASM);
    fs::write(&placed_wasm, b"corrupted").unwrap();

    service
        .reload_plugin(&store, builtin::BUILTIN_PI_SOURCE)
        .unwrap();

    assert_eq!(
        fs::read(&placed_wasm).unwrap(),
        builtin::PI_WASM.to_vec(),
        "宿主副本已按内嵌字节恢复"
    );
    let views = service.list(&store).unwrap();
    let view = plugin_view(&views, builtin::BUILTIN_PI_ID);
    assert_eq!(view.error, None);
    assert!(
        plugin_entry(&snapshot(&store).plugins, builtin::BUILTIN_PI_ID).enabled,
        "配置条目未被改动"
    );
}

/// 全新主目录：内置插件与其余来源同一安装管线（落位 + 写条目），且可重复启动。
#[test]
fn startup_installs_missing_builtin_plugin_like_any_other_source() {
    let home = temp_home();
    let store = store_at(home.path());
    let maestro_paths = MaestroPaths::get();
    let service = test_service(home.path());

    service.startup(&store).unwrap();

    let plugins = snapshot(&store).plugins;
    assert_eq!(
        plugins.len(),
        builtin::BUILTINS.len(),
        "启动时补装缺失的内置插件"
    );
    let pi = plugin_entry(&plugins, builtin::BUILTIN_PI_ID);
    assert_eq!(pi.source, builtin::BUILTIN_PI_SOURCE);
    assert!(pi.enabled);
    assert!(
        maestro_paths
            .plugin_dir("pi")
            .join(PLACED_MANIFEST)
            .exists()
            && maestro_paths.plugin_dir("pi").join(PLACED_WASM).exists(),
        "内置插件与其余来源一样落位到磁盘"
    );

    let views = service.list(&store).unwrap();
    assert_eq!(views.len(), builtin::BUILTINS.len());
    let view = plugin_view(&views, builtin::BUILTIN_PI_ID);
    assert!(view.builtin);
    assert_eq!(view.error, None, "{:?}", view.error);

    // 再次启动：条目已存在，不重复安装，注册表照常重建。
    service.startup(&store).unwrap();
    assert_eq!(
        snapshot(&store).plugins.len(),
        builtin::BUILTINS.len(),
        "重复启动不产生重复条目"
    );
    let views = service.list(&store).unwrap();
    assert_eq!(plugin_view(&views, builtin::BUILTIN_PI_ID).error, None);
}

/// 内置来源按 source 精确分派（issue #56）：此前任何 `builtin:` 来源都返回 pi 的
/// manifest 与 wasm，`builtin:claude-code` 会静默落位成 pi。这条回归钉住分派行为。
#[test]
fn builtin_source_dispatches_to_its_own_manifest_and_wasm() {
    let home = temp_home();
    let store = store_at(home.path());
    let maestro_paths = MaestroPaths::get();
    let service = test_service(home.path());

    service.startup(&store).unwrap();

    let plugins = snapshot(&store).plugins;
    let ids: BTreeSet<&str> = plugins.iter().map(|entry| entry.id.as_str()).collect();
    assert_eq!(
        ids,
        BTreeSet::from(["pi", "claude-code"]),
        "内置插件 id 集合"
    );
    assert_ne!(
        builtin::CLAUDE_CODE_WASM,
        builtin::PI_WASM,
        "两个内置产物必须是各自编译的 wasm"
    );

    // builtin:claude-code 落位的是 Claude Code 自己的 manifest 与 wasm。
    let dir = maestro_paths.plugin_dir(builtin::BUILTIN_CLAUDE_CODE_ID);
    assert_eq!(
        fs::read_to_string(dir.join(PLACED_MANIFEST)).unwrap(),
        builtin::CLAUDE_CODE_MANIFEST_JSON,
        "落位 manifest 为 Claude Code 自己的 manifest"
    );
    assert_eq!(
        fs::read(dir.join(PLACED_WASM)).unwrap(),
        builtin::CLAUDE_CODE_WASM.to_vec(),
        "落位 wasm 为 Claude Code 自己的产物"
    );

    // 未知的 builtin: 来源报错，不回落（回落会把别人的产物落位到该来源名下）。
    let err = service.add_plugin(&store, "builtin:ghost").unwrap_err();
    assert!(err.contains("未知的内置插件来源"), "{err}");
    assert!(
        !maestro_paths.plugin_dir("ghost").exists(),
        "未知来源不得落位"
    );
}

/// 逐插件报告：applied / failed / skipped 分类，单插件失败不影响其它插件。
#[test]
fn write_providers_reports_per_plugin_and_isolates_failures() {
    use crate::provider::Protocol;

    let home = temp_home();
    let store = store_at(home.path());
    let fetcher = https_stub("pi2");
    fetcher.serve(
        "https://example.com/pi3/manifest.json",
        manifest_json("pi3", "$HOME/.pi3", WASM_URL),
    );
    let service = stub_service(home.path(), fetcher);

    service.add_plugin(&store, MANIFEST_URL).unwrap();
    service
        .add_plugin(&store, "https://example.com/pi3/manifest.json")
        .unwrap();
    service
        .set_enabled(&store, "https://example.com/pi3/manifest.json", false)
        .unwrap();

    // 直接注入注册表的两种异常态：坏 wasm 的 Loaded 与显式 Error。
    // 装载走服务同一套平台目录（基准落在临时主目录，不读真实环境）。
    let bad = PlacedPlugin::new(
        &home.path().join("pibad"),
        manifest_json("pibad", "$HOME/.pibad", "plugin.wasm"),
        b"not a wasm component",
    )
    .load()
    .unwrap();
    let mut entries = service.entries.write().unwrap();
    entries.insert("pibad".to_owned(), PluginState::Loaded(Arc::new(bad)));
    entries.insert(
        "pierr".to_owned(),
        PluginState::Error("配置已损坏".to_owned()),
    );
    drop(entries);

    let providers = BTreeMap::from([(
        "gateway".to_owned(),
        Provider {
            base_url: BTreeMap::from([(
                Protocol::OpenaiCompletions,
                "https://api.example.com/v1".to_owned(),
            )]),
            ..Provider::default()
        },
    )]);

    let reports = service
        .write_providers(&providers, &BTreeMap::new())
        .unwrap();

    let by_id: HashMap<&str, &PluginApplyReport> = reports
        .iter()
        .map(|report| (report.id.as_str(), report))
        .collect();
    assert_eq!(reports.len(), 4, "每个注册表条目一份报告");
    let applied = by_id.get("pi2").unwrap();
    assert_eq!(applied.status, "applied");
    assert_eq!(applied.files.len(), 1);
    let reported = Path::new(&applied.files[0]);
    assert!(
        reported.is_absolute() && reported.ends_with("agent/models.json"),
        "files 是宿主绝对路径：{:?}",
        applied.files
    );
    let failed = by_id.get("pibad").unwrap();
    assert_eq!(failed.status, "failed");
    assert!(
        failed
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("不是有效的 WASM 组件"),
        "{:?}",
        failed.reason
    );
    assert_eq!(by_id.get("pi3").unwrap().status, "skipped", "禁用态跳过");
    assert_eq!(
        by_id.get("pi3").unwrap().reason.as_deref(),
        Some("插件已禁用")
    );
    assert_eq!(by_id.get("pierr").unwrap().status, "skipped", "错误态跳过");
    assert_eq!(
        by_id.get("pierr").unwrap().reason.as_deref(),
        Some("配置已损坏")
    );
}

/// L2 金丝雀（ADR 0008）：以带唯一 canary key 的 provider 跑真实投影路径
/// （内置 pi 组件实例化 + 写投影），在捕获窗口内断言全部输出不含该值——
/// 把「api_key 绝不进日志」变成可执行断言。
#[test]
fn projection_path_never_logs_the_api_key() {
    use crate::provider::Protocol;

    let window = crate::logging::capture::capture_window();
    let home = temp_home();
    let store = store_at(home.path());
    let service = test_service(home.path());
    service.startup(&store).unwrap();

    let canary = "sk-canary-1eaf9c0b";
    let providers = BTreeMap::from([(
        "gateway".to_owned(),
        Provider {
            api_key: canary.to_owned(),
            base_url: BTreeMap::from([(
                Protocol::OpenaiCompletions,
                "https://api.example.com/v1".to_owned(),
            )]),
            ..Provider::default()
        },
    )]);

    let reports = service
        .write_providers(&providers, &BTreeMap::new())
        .unwrap();

    assert_eq!(reports[0].status, "applied", "{:?}", reports[0].reason);
    let logs = window.lines();
    // 先证捕获窗口确实覆盖了这条真实投影路径，canary 断言才有意义。
    assert!(
        logs.iter()
            .any(|line| line.ends_with("projecting providers: id=pi")),
        "捕获输出应包含投影步骤的 DEBUG 行：{logs:?}"
    );
    assert!(
        !logs.iter().any(|line| line.contains(canary)),
        "api_key 绝不进入日志输出：{logs:?}"
    );
}

/// L2 金丝雀（ADR 0008）：凭证不止 api_key——自定义 header 的**值**合同上也可含
/// 凭证（WIT 合同与 `crates/maestro-plugin-sdk/README.md` 同此声明）。同一条真实
/// 投影路径上换用唯一 canary header 值再跑一遍，窗口内断言输出不含该值。
#[test]
fn projection_path_never_logs_custom_header_values() {
    use crate::provider::Protocol;

    let window = crate::logging::capture::capture_window();
    let home = temp_home();
    let store = store_at(home.path());
    let service = test_service(home.path());
    service.startup(&store).unwrap();

    let canary = "gw-canary-7c41d8e2";
    let providers = BTreeMap::from([(
        "gateway".to_owned(),
        Provider {
            custom_header: BTreeMap::from([("X-Gateway-Key".to_owned(), canary.to_owned())]),
            base_url: BTreeMap::from([(
                Protocol::OpenaiCompletions,
                "https://api.example.com/v1".to_owned(),
            )]),
            ..Provider::default()
        },
    )]);

    let reports = service
        .write_providers(&providers, &BTreeMap::new())
        .unwrap();

    assert_eq!(reports[0].status, "applied", "{:?}", reports[0].reason);
    let logs = window.lines();
    // 先证捕获窗口确实覆盖了这条真实投影路径，canary 断言才有意义。
    assert!(
        logs.iter()
            .any(|line| line.ends_with("projecting providers: id=pi")),
        "捕获输出应包含投影步骤的 DEBUG 行：{logs:?}"
    );
    assert!(
        !logs.iter().any(|line| line.contains(canary)),
        "自定义 header 值绝不进入日志输出：{logs:?}"
    );
}

/// 投影前宿主先插值（ADR 0015）：占位符在调用插件前替换为变量值，
/// 插件只收到替换后的字面值。
#[test]
fn write_providers_interpolates_placeholders_before_projection() {
    use crate::provider::Protocol;

    let home = temp_home();
    let store = store_at(home.path());
    let service = test_service(home.path());
    service.startup(&store).unwrap();

    let providers = BTreeMap::from([(
        "gateway".to_owned(),
        Provider {
            base_url: BTreeMap::from([(
                Protocol::OpenaiCompletions,
                "https://${HOST}/v1".to_owned(),
            )]),
            api_key: "sk-literal".to_owned(),
            ..Provider::default()
        },
    )]);
    let variables = BTreeMap::from([("HOST".to_owned(), "api.example.com".to_owned())]);

    let reports = service.write_providers(&providers, &variables).unwrap();

    assert_eq!(reports[0].status, "applied", "{:?}", reports[0].reason);
    let written: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(home.path().join(".pi").join("agent").join("models.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        written["providers"]["gateway"]["baseUrl"],
        "https://api.example.com/v1"
    );
    assert_eq!(
        written["providers"]["gateway"]["apiKey"], "sk-literal",
        "api_key 原样投影、不参与插值"
    );
}

/// 投影按插件读取插件配置（ADR 0018）：全局变量表 ⊕ 该插件的 `variables` 覆盖
/// 后再插值——同一占位符在不同插件下解析出不同值，未覆盖的沿用全局默认；
/// `form` 段的存在不影响投影（宿主只把它递送给组件，不参与插值）。
#[test]
fn write_providers_merges_plugin_variable_overrides_per_plugin() {
    use crate::provider::Protocol;

    let home = temp_home();
    let store = store_at(home.path());
    let service = stub_service(home.path(), https_stub("pi2"));
    service.startup(&store).unwrap();
    // https 来源由替身 fetcher 显式安装；startup 只补装缺失的内置插件。
    service.add_plugin(&store, MANIFEST_URL).unwrap();
    let variables = BTreeMap::from([("HOST".to_owned(), "api.example.com".to_owned())]);
    store
        .write()
        .unwrap()
        .set_variables(variables.clone())
        .unwrap();

    let providers = BTreeMap::from([(
        "gateway".to_owned(),
        Provider {
            base_url: BTreeMap::from([(
                Protocol::OpenaiCompletions,
                "https://${HOST}/v1".to_owned(),
            )]),
            ..Provider::default()
        },
    )]);

    // 未覆盖的内置 pi 沿用全局默认；pi2 覆盖同一变量解析出不同值。
    service
        .set_config(
            &store,
            MANIFEST_URL,
            &PluginConfig {
                variables: BTreeMap::from([("HOST".to_owned(), "gateway.local".to_owned())]),
                form: Some(serde_json::json!({"model": "kimi"})),
            },
        )
        .unwrap();

    let reports = service.write_providers(&providers, &variables).unwrap();

    let by_id: HashMap<&str, &PluginApplyReport> = reports
        .iter()
        .map(|report| (report.id.as_str(), report))
        .collect();
    assert_eq!(by_id.get("pi").unwrap().status, "applied", "{:?}", reports);
    assert_eq!(by_id.get("pi2").unwrap().status, "applied", "{:?}", reports);
    let written: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(home.path().join(".pi").join("agent").join("models.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        written["providers"]["gateway"]["baseUrl"], "https://api.example.com/v1",
        "未覆盖的插件沿用全局变量表"
    );
    let overridden: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(home.path().join(".pi2").join("agent").join("models.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        overridden["providers"]["gateway"]["baseUrl"], "https://gateway.local/v1",
        "同一占位符在覆盖插件下解析出不同值"
    );
}

/// 任一字段解析失败即整插件 failed：不调用插件、不写任何文件，
/// 原因含 slug + 字段名 + 变量名，不含值与密钥（ADR 0015）。
#[test]
fn write_providers_fails_the_whole_plugin_and_writes_nothing_on_interpolation_error() {
    use crate::provider::Protocol;

    let home = temp_home();
    let store = store_at(home.path());
    let service = test_service(home.path());
    service.startup(&store).unwrap();

    let providers = BTreeMap::from([(
        "gateway".to_owned(),
        Provider {
            base_url: BTreeMap::from([(
                Protocol::OpenaiCompletions,
                "https://${UNDEFINED_VAR}/v1".to_owned(),
            )]),
            api_key: "sk-canary-must-not-leak".to_owned(),
            ..Provider::default()
        },
    )]);

    let reports = service
        .write_providers(&providers, &BTreeMap::new())
        .unwrap();

    assert_eq!(reports[0].status, "failed");
    let reason = reports[0].reason.as_deref().unwrap();
    assert!(reason.contains("gateway"), "{reason}");
    assert!(reason.contains("base_url.openai-completions"), "{reason}");
    assert!(reason.contains("UNDEFINED_VAR"), "{reason}");
    assert!(
        !reason.contains("sk-canary-must-not-leak"),
        "失败原因不得携带值与密钥：{reason}"
    );
    assert!(
        !home.path().join(".pi").join("agent").exists(),
        "插值失败不得实例化插件、不得写任何文件"
    );
}

/// 插件配置读取/解析失败即整插件 failed（ADR 0018，不静默降级为空配置）：
/// 不实例化、不调用插件、不写任何文件，原因含损坏文件的路径。
#[test]
fn write_providers_fails_the_whole_plugin_when_its_plugin_config_is_corrupt() {
    use crate::provider::Protocol;

    let home = temp_home();
    let store = store_at(home.path());
    let service = test_service(home.path());
    service.startup(&store).unwrap();
    let maestro_paths = MaestroPaths::get();
    fs::write(
        maestro_paths.plugin_dir("pi").join(PLACED_CONFIG),
        "{ not json",
    )
    .unwrap();

    let providers = BTreeMap::from([(
        "gateway".to_owned(),
        Provider {
            base_url: BTreeMap::from([(
                Protocol::OpenaiCompletions,
                "https://api.example.com/v1".to_owned(),
            )]),
            ..Provider::default()
        },
    )]);

    let reports = service
        .write_providers(&providers, &BTreeMap::new())
        .unwrap();

    let report = plugin_report(&reports, "pi");
    assert_eq!(report.status, "failed");
    let reason = report.reason.as_deref().unwrap();
    assert!(reason.contains("插件配置已损坏"), "{reason}");
    assert!(
        reason.contains(
            &maestro_paths
                .plugin_dir("pi")
                .join(PLACED_CONFIG)
                .display()
                .to_string()
        ),
        "失败原因应含损坏文件的路径：{reason}"
    );
    assert!(
        !home.path().join(".pi").join("agent").exists(),
        "插件配置损坏不得实例化插件、不得写任何文件"
    );
}

/// 先插值、后映射端点：解析为空串的端点按未配置处理（ADR 0015），
/// 走既有的合法跳过路径而非失败。
#[test]
fn endpoint_that_interpolates_to_empty_counts_as_unconfigured() {
    use crate::provider::Protocol;

    let home = temp_home();
    let store = store_at(home.path());
    let service = test_service(home.path());
    service.startup(&store).unwrap();

    let providers = BTreeMap::from([(
        "gateway".to_owned(),
        Provider {
            base_url: BTreeMap::from([(Protocol::OpenaiCompletions, "${EMPTY}".to_owned())]),
            ..Provider::default()
        },
    )]);
    let variables = BTreeMap::from([("EMPTY".to_owned(), String::new())]);

    let reports = service.write_providers(&providers, &variables).unwrap();

    assert_eq!(reports[0].status, "applied", "{:?}", reports[0].reason);
    assert_eq!(reports[0].skipped.len(), 1);
    assert_eq!(reports[0].skipped[0].slug, "gateway");
    assert_eq!(reports[0].skipped[0].reason, "未配置任何协议端点");
}

/// 新装插件开箱即用（ADR 0018）：config.json 不存在视为「无覆盖、无表单」，
/// 而非报错。
#[test]
fn get_plugin_config_returns_no_overrides_and_no_form_for_a_freshly_installed_plugin() {
    let home = temp_home();
    let store = store_at(home.path());
    let service = test_service(home.path());
    service.startup(&store).unwrap();

    let config = service
        .get_config(&store, builtin::BUILTIN_PI_SOURCE)
        .unwrap();

    assert!(config.variables.is_empty(), "{:?}", config.variables);
    assert_eq!(config.form, None);
}

/// 写入即整包替换（ADR 0018）：variables 覆盖与 form 落到 `<id>/config.json`，
/// 结构为 `{ "variables": {…}, "form": {…} }`，经读命令原样回读。
#[test]
fn set_plugin_config_persists_variables_and_form_and_reads_back() {
    let home = temp_home();
    let store = store_at(home.path());
    let maestro_paths = MaestroPaths::get();
    let service = test_service(home.path());
    service.startup(&store).unwrap();
    store
        .write()
        .unwrap()
        .set_variables(BTreeMap::from([(
            "HOST".to_owned(),
            "api.example.com".to_owned(),
        )]))
        .unwrap();

    let config = PluginConfig {
        variables: BTreeMap::from([("HOST".to_owned(), "gateway.local".to_owned())]),
        form: Some(serde_json::json!({"model": "kimi"})),
    };
    service
        .set_config(&store, builtin::BUILTIN_PI_SOURCE, &config)
        .unwrap();

    let config_path = maestro_paths.plugin_dir("pi").join(PLACED_CONFIG);
    let raw: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(raw["variables"]["HOST"], "gateway.local");
    assert_eq!(raw["form"]["model"], "kimi");
    assert_eq!(
        service
            .get_config(&store, builtin::BUILTIN_PI_SOURCE)
            .unwrap(),
        config
    );
}

/// 覆盖的变量名必须已在全局变量表中声明：未声明即报错，且不落盘；
/// 报错含变量名（标识符），不含变量值。
#[test]
fn set_plugin_config_rejects_variables_not_declared_in_the_global_table() {
    let home = temp_home();
    let store = store_at(home.path());
    let maestro_paths = MaestroPaths::get();
    let service = test_service(home.path());
    service.startup(&store).unwrap();

    let config = PluginConfig {
        variables: BTreeMap::from([("UNDECLARED".to_owned(), "sk-canary".to_owned())]),
        form: None,
    };
    let err = service
        .set_config(&store, builtin::BUILTIN_PI_SOURCE, &config)
        .unwrap_err();

    assert!(err.contains("未在全局变量表中声明"), "{err}");
    assert!(err.contains("UNDECLARED"), "{err}");
    assert!(!err.contains("sk-canary"), "报错不得携带变量值：{err}");
    assert!(
        !maestro_paths.plugin_dir("pi").join(PLACED_CONFIG).exists(),
        "被拒绝的写入不得落盘"
    );
}

/// 插件配置损坏时报错并给出可读原因（含文件路径），不静默降级为「无覆盖」。
#[test]
fn get_plugin_config_reports_corruption_instead_of_degrading() {
    let home = temp_home();
    let store = store_at(home.path());
    let maestro_paths = MaestroPaths::get();
    let service = test_service(home.path());
    service.startup(&store).unwrap();
    fs::write(
        maestro_paths.plugin_dir("pi").join(PLACED_CONFIG),
        "{ not json",
    )
    .unwrap();

    let err = service
        .get_config(&store, builtin::BUILTIN_PI_SOURCE)
        .unwrap_err();

    assert!(err.contains("插件配置已损坏"), "{err}");
    assert!(
        err.contains(
            &maestro_paths
                .plugin_dir("pi")
                .join(PLACED_CONFIG)
                .display()
                .to_string()
        ),
        "报错应含文件路径：{err}"
    );
}

/// 端到端（真实内置 pi 产物）：经写命令落盘的插件配置在重载后仍在——
/// 逐文件覆盖只替换 manifest.json 与 plugin.wasm 两个宿主产物（ADR 0018），
/// 本测试覆盖「经服务层写入」的完整路径。
#[test]
fn set_plugin_config_survives_a_plugin_reload() {
    let home = temp_home();
    let store = store_at(home.path());
    let service = test_service(home.path());
    service.startup(&store).unwrap();
    store
        .write()
        .unwrap()
        .set_variables(BTreeMap::from([(
            "HOST".to_owned(),
            "api.example.com".to_owned(),
        )]))
        .unwrap();

    let config = PluginConfig {
        variables: BTreeMap::from([("HOST".to_owned(), "gateway.local".to_owned())]),
        form: Some(serde_json::json!({"model": "kimi"})),
    };
    service
        .set_config(&store, builtin::BUILTIN_PI_SOURCE, &config)
        .unwrap();
    service
        .reload_plugin(&store, builtin::BUILTIN_PI_SOURCE)
        .unwrap();

    assert_eq!(
        service
            .get_config(&store, builtin::BUILTIN_PI_SOURCE)
            .unwrap(),
        config,
        "重载不冲掉经命令写入的插件配置"
    );
}

/// 插件返回的路径必须是 config_dir 内的相对路径：绝对路径与 `..` 一律拒绝（fail-loud），
/// 它们是预开放目录之外的写入面，不得被回报成一次成功投影。
#[test]
fn project_files_rejects_paths_outside_the_config_dir() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join(".pi");
    let absolute = temp.path().join("outside.json").display().to_string();

    assert_eq!(
        project_files(&config_dir, vec!["agent/models.json".to_owned()]).unwrap(),
        vec![config_dir.join("agent/models.json").display().to_string()]
    );
    for file in [
        absolute.as_str(),
        "../outside.json",
        "agent/../../etc/passwd",
    ] {
        let err = project_files(&config_dir, vec![file.to_owned()]).unwrap_err();
        assert!(err.contains(file), "{file} 应被拒绝：{err}");
    }
}
