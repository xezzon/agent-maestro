use std::path::{Path, PathBuf};

use tempfile::TempDir;

/// 校验插件来源：v1 仅匿名 HTTPS。
///
/// 拒绝非 HTTPS 协议与携带用户名/密码的 URL（`https://user:pass@host/…`）；
/// 本地路径仅在测试构建下放行（`cargo test` 端到端走真实克隆代码）。
/// `clone_to_staging` 会再次调用本函数，持久化的非法 source 无法绕过。
pub fn validate_source(source: &str) -> Result<(), String> {
    if let Some(rest) = source.strip_prefix("https://") {
        if rest.is_empty() {
            return Err("Git 来源仅支持匿名 HTTPS 地址".to_owned());
        }
        if rest.chars().any(char::is_whitespace) {
            return Err("Git 来源地址不合法".to_owned());
        }
        // host 段（首个 `/` 之前）出现 `@` 即携带了 user[:password] 凭据。
        let host = rest.split('/').next().unwrap_or(rest);
        if host.contains('@') {
            return Err("Git 来源仅支持匿名 HTTPS 地址，不得携带用户名或密码".to_owned());
        }
        return Ok(());
    }
    #[cfg(test)]
    {
        let is_local_path = !source.contains("://")
            && !source.starts_with("git@")
            && Path::new(source).is_absolute();
        if is_local_path {
            return Ok(());
        }
    }
    Err("Git 来源仅支持匿名 HTTPS 地址".to_owned())
}

/// 克隆 Git 来源到临时暂存目录（插件安装目录内，保证与落位目标同文件系统可 rename）。
///
/// 默认分支，repo 根即插件根（见 issue #34）。
/// 暂存目录随 `TempDir` drop 自动清理，调用方成功后须先 `keep()` 再落位。
pub fn clone_to_staging(source: &str, plugins_root: &Path) -> Result<TempDir, String> {
    validate_source(source)?;
    std::fs::create_dir_all(plugins_root).map_err(|e| format!("创建插件安装目录失败：{e}"))?;
    let staging = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(plugins_root)
        .map_err(|e| format!("创建克隆暂存目录失败：{e}"))?;
    git2::build::RepoBuilder::new()
        .clone(source, staging.path())
        .map_err(|e| format!("克隆插件仓库失败：{e}"))?;
    Ok(staging)
}

/// 读取并校验暂存目录中的 manifest（repo 根即插件根）。
pub fn read_staging_manifest(staging: &Path) -> Result<crate::plugin::manifest::Manifest, String> {
    let text = std::fs::read_to_string(staging.join("manifest.json"))
        .map_err(|_| "插件仓库根目录缺少 manifest.json".to_owned())?;
    crate::plugin::manifest::parse_manifest(&text)
}

/// 把暂存目录落位为 `plugins_root/<id>`；已存在的旧目录一并替换（更新语义）。
///
/// 旧目录先移到备份位置，placement 成功后才删除备份：placement 失败时恢复
/// 旧目录，保证更新失败后旧版本仍然可用。
pub fn install_staging(staging: TempDir, target_root: &Path, id: &str) -> Result<PathBuf, String> {
    let target = target_root.join(id);
    let staging = staging.keep();
    if target.exists() {
        // 备份名以 `.` 开头，与插件 id（小写字母开头）不会冲突。
        let backup = target_root.join(format!(".backup-{id}"));
        std::fs::rename(&target, &backup).map_err(|e| format!("备份旧插件目录失败：{e}"))?;
        match std::fs::rename(&staging, &target) {
            Ok(()) => {
                let _ = std::fs::remove_dir_all(&backup);
            }
            Err(e) => {
                let _ = std::fs::rename(&backup, &target);
                return Err(format!("落位插件目录失败：{e}"));
            }
        }
    } else {
        std::fs::rename(&staging, &target).map_err(|e| format!("落位插件目录失败：{e}"))?;
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::super::{
        builtin,
        testutil::{store_at, temp_home, test_service},
    };
    use super::validate_source;
    use git2::Repository;
    use std::{fs, path::Path, sync::Mutex};

    /// 以本地临时目录作 remote（git2 支持本地路径，离线可跑）。
    fn init_repo(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        let repo = Repository::init(dir).unwrap();
        commit_all(&repo);
    }

    fn commit_all(repo: &Repository) {
        let mut index = repo.index().unwrap();
        index
            .add_all(["."].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("test", "test@example.com").unwrap();
        let parents: Vec<_> = match repo.head() {
            Ok(head) => vec![repo.find_commit(head.target().unwrap()).unwrap()],
            Err(_) => Vec::new(),
        };
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, "commit", &tree, &parent_refs)
            .unwrap();
    }

    /// repo 根即插件根：manifest.json + plugin.wasm。
    fn write_plugin_files(dir: &Path, id: &str, name: &str) {
        fs::write(
            dir.join("manifest.json"),
            format!(
                r#"{{ "id": "{id}", "name": "{name}", "tool": "sample", "config_dir": "~/.sample", "entry": "plugin.wasm" }}"#
            ),
        )
        .unwrap();
        fs::write(dir.join("plugin.wasm"), builtin::PI_WASM).unwrap();
    }

    #[test]
    fn add_installs_plugin_from_git_source_and_backfills_id() {
        let home = temp_home();
        let repo_dir = tempfile::tempdir().unwrap();
        write_plugin_files(repo_dir.path(), "sample", "Sample");
        init_repo(repo_dir.path());

        let store = Mutex::new(store_at(home.path()));
        let service = test_service(home.path());

        service
            .add(&store, repo_dir.path().to_str().unwrap())
            .unwrap();

        // 配置条目回填 id，插件落位安装目录并加载成功。
        let guard = store.lock().unwrap();
        let entry = &guard.get().unwrap().plugins[0];
        assert_eq!(entry.id.as_deref(), Some("sample"));
        drop(guard);
        assert!(
            home.path()
                .join(".maestro/plugins/sample/manifest.json")
                .exists(),
            "插件落位 ~/.maestro/plugins/<id>"
        );
        let views = service.list();
        assert_eq!(views[0].status, "loaded");
        assert_eq!(views[0].name.as_deref(), Some("Sample"));
        assert!(!views[0].builtin);
    }

    #[test]
    fn add_failure_keeps_entry_and_reports_reason() {
        let home = temp_home();
        let repo_dir = tempfile::tempdir().unwrap();
        // 与内置插件 id 冲突：落位前的冲突检查拒绝安装。
        write_plugin_files(repo_dir.path(), "pi", "Pi Clone");
        init_repo(repo_dir.path());

        let store = Mutex::new(store_at(home.path()));
        let service = test_service(home.path());
        service.startup(&store);

        let err = service
            .add(&store, repo_dir.path().to_str().unwrap())
            .unwrap_err();
        assert!(err.contains("已被其他来源占用"), "{err}");

        // 条目保留可重试，插件进错误态并展示原因。
        assert_eq!(store.lock().unwrap().get().unwrap().plugins.len(), 2);
        let views = service.list();
        let added = views.iter().find(|v| !v.builtin).unwrap();
        assert_eq!(added.status, "error");
        assert!(added.error.as_deref().unwrap().contains("占用"));
        assert!(!home.path().join(".maestro/plugins/pi").exists());
    }

    #[test]
    fn duplicate_source_add_is_rejected() {
        let home = temp_home();
        let repo_dir = tempfile::tempdir().unwrap();
        write_plugin_files(repo_dir.path(), "sample", "Sample");
        init_repo(repo_dir.path());

        let store = Mutex::new(store_at(home.path()));
        let service = test_service(home.path());
        service
            .add(&store, repo_dir.path().to_str().unwrap())
            .unwrap();

        let err = service
            .add(&store, repo_dir.path().to_str().unwrap())
            .unwrap_err();
        assert!(err.contains("已存在相同来源的插件"), "{err}");
        assert_eq!(store.lock().unwrap().get().unwrap().plugins.len(), 1);
    }

    #[test]
    fn update_pulls_latest_version_from_upstream() {
        let home = temp_home();
        let repo_dir = tempfile::tempdir().unwrap();
        write_plugin_files(repo_dir.path(), "sample", "Sample");
        init_repo(repo_dir.path());

        let store = Mutex::new(store_at(home.path()));
        let service = test_service(home.path());
        let source = repo_dir.path().to_str().unwrap();
        service.add(&store, source).unwrap();

        // 上游发布新版本：改 manifest 后提交。
        write_plugin_files(repo_dir.path(), "sample", "Sample v2");
        commit_all(&Repository::open(repo_dir.path()).unwrap());
        service.update(&store, source).unwrap();

        let views = service.list();
        assert_eq!(
            views[0].name.as_deref(),
            Some("Sample v2"),
            "更新后加载新版本"
        );
    }

    #[test]
    fn update_with_renamed_upstream_id_is_rejected_and_keeps_old_version() {
        let home = temp_home();
        let repo_dir = tempfile::tempdir().unwrap();
        write_plugin_files(repo_dir.path(), "sample", "Sample");
        init_repo(repo_dir.path());

        let store = Mutex::new(store_at(home.path()));
        let service = test_service(home.path());
        let source = repo_dir.path().to_str().unwrap();
        service.add(&store, source).unwrap();

        // 上游把 id 改成别的名字：更新报错并保持旧状态。
        write_plugin_files(repo_dir.path(), "renamed", "Renamed");
        commit_all(&Repository::open(repo_dir.path()).unwrap());
        let err = service.update(&store, source).unwrap_err();
        assert!(err.contains("id 已变更"), "{err}");
        assert_eq!(
            service.list()[0].name.as_deref(),
            Some("Sample"),
            "旧版本继续可用"
        );
    }

    #[test]
    fn update_failure_keeps_old_version_usable() {
        let home = temp_home();
        let repo_dir = tempfile::tempdir().unwrap();
        write_plugin_files(repo_dir.path(), "sample", "Sample");
        init_repo(repo_dir.path());

        let store = Mutex::new(store_at(home.path()));
        let service = test_service(home.path());
        let source = repo_dir.path().to_str().unwrap();
        service.add(&store, source).unwrap();

        // remote 消失（网络类失败）：更新失败，旧版本继续可用。
        fs::remove_dir_all(repo_dir.path()).unwrap();
        assert!(service.update(&store, source).is_err());
        assert_eq!(service.list()[0].status, "loaded");
        assert_eq!(service.list()[0].name.as_deref(), Some("Sample"));
    }

    #[test]
    fn update_with_incompatible_wasm_keeps_old_version() {
        let home = temp_home();
        let repo_dir = tempfile::tempdir().unwrap();
        write_plugin_files(repo_dir.path(), "sample", "Sample");
        init_repo(repo_dir.path());

        let store = Mutex::new(store_at(home.path()));
        let service = test_service(home.path());
        let source = repo_dir.path().to_str().unwrap();
        service.add(&store, source).unwrap();

        // 上游发布接口不兼容的新版本（core module 连组件都不是）：
        // 落位前校验失败，旧目录保持原样，旧版本继续可用。
        fs::write(repo_dir.path().join("plugin.wasm"), br#"(module)"#).unwrap();
        commit_all(&Repository::open(repo_dir.path()).unwrap());
        let err = service.update(&store, source).unwrap_err();
        assert!(err.contains("不是有效的 WASM 组件"), "{err}");
        assert_eq!(service.list()[0].status, "loaded");
        assert_eq!(service.list()[0].name.as_deref(), Some("Sample"));
        assert!(
            home.path()
                .join(".maestro/plugins/sample/plugin.wasm")
                .exists(),
            "旧插件目录未被损坏的新版本替换"
        );
    }

    #[test]
    fn remove_deletes_config_entry_and_plugin_directory() {
        let home = temp_home();
        let repo_dir = tempfile::tempdir().unwrap();
        write_plugin_files(repo_dir.path(), "sample", "Sample");
        init_repo(repo_dir.path());

        let store = Mutex::new(store_at(home.path()));
        let service = test_service(home.path());
        service
            .add(&store, repo_dir.path().to_str().unwrap())
            .unwrap();

        service
            .remove(
                &mut store.lock().unwrap(),
                repo_dir.path().to_str().unwrap(),
            )
            .unwrap();

        assert!(
            store.lock().unwrap().get().unwrap().plugins.is_empty(),
            "配置条目一并清理"
        );
        assert!(
            !home.path().join(".maestro/plugins/sample").exists(),
            "插件目录一并清理"
        );
        assert!(service.list().is_empty());
    }

    #[test]
    fn remove_builtin_is_rejected() {
        let home = temp_home();
        let store = Mutex::new(store_at(home.path()));
        let service = test_service(home.path());
        service.startup(&store);

        let err = service
            .remove(&mut store.lock().unwrap(), builtin::BUILTIN_PI_SOURCE)
            .unwrap_err();
        assert!(err.contains("不可移除"), "{err}");
        assert_eq!(service.list().len(), 1, "内置插件仍可用");
    }

    #[test]
    fn git_source_must_be_anonymous_https() {
        assert!(validate_source("https://example.com/some-plugin.git").is_ok());
        assert!(validate_source("http://example.com/some-plugin.git").is_err());
        assert!(validate_source("git@github.com:user/repo.git").is_err());
        assert!(validate_source("file:///tmp/repo").is_err());
        assert!(validate_source("ssh://example.com/repo.git").is_err());
        assert!(validate_source("https://").is_err());
        assert!(validate_source("不是地址").is_err());
        assert!(validate_source("https://exa mple.com/repo.git").is_err());
    }

    #[test]
    fn git_source_with_credentials_is_rejected() {
        assert!(validate_source("https://user@example.com/repo.git").is_err());
        assert!(validate_source("https://user:pass@example.com/repo.git").is_err());
        // `@` 出现在路径段（host 段之后）不算凭据。
        assert!(validate_source("https://example.com/user@name/repo.git").is_ok());
    }

    #[test]
    fn local_paths_are_only_valid_in_test_builds() {
        let dir = tempfile::tempdir().unwrap();
        if cfg!(test) {
            assert!(validate_source(dir.path().to_str().unwrap()).is_ok());
        } else {
            assert!(validate_source(dir.path().to_str().unwrap()).is_err());
        }
    }

    fn provider_openai(url: &str, api_key: &str) -> crate::provider::Provider {
        crate::provider::Provider {
            base_url: crate::provider::Endpoints {
                openai_completions: Some(url.to_owned()),
                anthropic_messages: None,
            },
            api_key: api_key.to_owned(),
            models: vec![crate::provider::ModelEntry {
                id: "gpt-4o".to_owned(),
                display_name: Some("GPT-4o".to_owned()),
            }],
        }
    }

    /// 回归覆盖：Git 安装的插件（WasmSource::Disk）也必须能执行投影。
    #[test]
    fn apply_projects_providers_through_git_installed_plugin() {
        let home = temp_home();
        let repo_dir = tempfile::tempdir().unwrap();
        write_plugin_files(repo_dir.path(), "sample", "Sample");
        init_repo(repo_dir.path());

        let store = Mutex::new(store_at(home.path()));
        let service = test_service(home.path());
        service
            .add(&store, repo_dir.path().to_str().unwrap())
            .unwrap();

        store
            .lock()
            .unwrap()
            .create_provider("gateway", provider_openai("https://api.example.com/v1", ""))
            .unwrap();
        let providers = store.lock().unwrap().get().unwrap().providers.clone();

        let reports = service.apply(&providers);

        assert_eq!(reports.len(), 1);
        assert_eq!(
            reports[0].status, "applied",
            "reason: {:?}",
            reports[0].reason
        );
        assert_eq!(reports[0].id.as_deref(), Some("sample"));
        assert_eq!(reports[0].files, vec!["agent/models.json"]);
        assert!(
            home.path().join(".sample/agent/models.json").exists(),
            "投影落盘到插件声明的 config_dir"
        );
    }
}
