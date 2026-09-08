use std::path::{Path, PathBuf};

use tempfile::TempDir;

/// 克隆 Git 来源到临时暂存目录（插件安装目录内，保证与落位目标同文件系统可 rename）。
///
/// v1 仅匿名 HTTPS / 本地路径（本地路径仅用于测试端到端走真实克隆代码），
/// 默认分支，repo 根即插件根（见 issue #34）。
/// 暂存目录随 `TempDir` drop 自动清理，调用方成功后须先 `keep()` 再落位。
pub fn clone_to_staging(source: &str, plugins_root: &Path) -> Result<TempDir, String> {
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
pub fn install_staging(staging: TempDir, target_root: &Path, id: &str) -> Result<PathBuf, String> {
    let target = target_root.join(id);
    if target.exists() {
        std::fs::remove_dir_all(&target).map_err(|e| format!("清理旧插件目录失败：{e}"))?;
    }
    let staging = staging.keep();
    std::fs::rename(&staging, &target).map_err(|e| format!("落位插件目录失败：{e}"))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::super::{PluginService, builtin};
    use crate::store::Store;
    use git2::Repository;
    use std::{fs, path::Path};

    fn temp_home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn store_at(home: &Path) -> Store {
        Store::open(home.join(".maestro").join("config.json"))
    }

    fn test_service(home: &Path) -> PluginService {
        PluginService::new(Some(home.to_owned()))
    }

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

        let mut store = store_at(home.path());
        let service = test_service(home.path());

        service
            .add(&mut store, repo_dir.path().to_str().unwrap())
            .unwrap();

        // 配置条目回填 id，插件落位安装目录并加载成功。
        let entry = &store.get().unwrap().plugins[0];
        assert_eq!(entry.id.as_deref(), Some("sample"));
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

        let mut store = store_at(home.path());
        let service = test_service(home.path());
        service.startup(&mut store);

        let err = service
            .add(&mut store, repo_dir.path().to_str().unwrap())
            .unwrap_err();
        assert!(err.contains("已被其他来源占用"), "{err}");

        // 条目保留可重试，插件进错误态并展示原因。
        assert_eq!(store.get().unwrap().plugins.len(), 2);
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

        let mut store = store_at(home.path());
        let service = test_service(home.path());
        service
            .add(&mut store, repo_dir.path().to_str().unwrap())
            .unwrap();

        let err = service
            .add(&mut store, repo_dir.path().to_str().unwrap())
            .unwrap_err();
        assert!(err.contains("已存在相同来源的插件"), "{err}");
        assert_eq!(store.get().unwrap().plugins.len(), 1);
    }

    #[test]
    fn update_pulls_latest_version_from_upstream() {
        let home = temp_home();
        let repo_dir = tempfile::tempdir().unwrap();
        write_plugin_files(repo_dir.path(), "sample", "Sample");
        init_repo(repo_dir.path());

        let mut store = store_at(home.path());
        let service = test_service(home.path());
        let source = repo_dir.path().to_str().unwrap();
        service.add(&mut store, source).unwrap();

        // 上游发布新版本：改 manifest 后提交。
        write_plugin_files(repo_dir.path(), "sample", "Sample v2");
        commit_all(&Repository::open(repo_dir.path()).unwrap());
        service.update(&mut store, source).unwrap();

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

        let mut store = store_at(home.path());
        let service = test_service(home.path());
        let source = repo_dir.path().to_str().unwrap();
        service.add(&mut store, source).unwrap();

        // 上游把 id 改成别的名字：更新报错并保持旧状态。
        write_plugin_files(repo_dir.path(), "renamed", "Renamed");
        commit_all(&Repository::open(repo_dir.path()).unwrap());
        let err = service.update(&mut store, source).unwrap_err();
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

        let mut store = store_at(home.path());
        let service = test_service(home.path());
        let source = repo_dir.path().to_str().unwrap();
        service.add(&mut store, source).unwrap();

        // remote 消失（网络类失败）：更新失败，旧版本继续可用。
        fs::remove_dir_all(repo_dir.path()).unwrap();
        assert!(service.update(&mut store, source).is_err());
        assert_eq!(service.list()[0].status, "loaded");
        assert_eq!(service.list()[0].name.as_deref(), Some("Sample"));
    }

    #[test]
    fn remove_deletes_config_entry_and_plugin_directory() {
        let home = temp_home();
        let repo_dir = tempfile::tempdir().unwrap();
        write_plugin_files(repo_dir.path(), "sample", "Sample");
        init_repo(repo_dir.path());

        let mut store = store_at(home.path());
        let service = test_service(home.path());
        service
            .add(&mut store, repo_dir.path().to_str().unwrap())
            .unwrap();

        service
            .remove(&mut store, repo_dir.path().to_str().unwrap())
            .unwrap();

        assert!(store.get().unwrap().plugins.is_empty(), "配置条目一并清理");
        assert!(
            !home.path().join(".maestro/plugins/sample").exists(),
            "插件目录一并清理"
        );
        assert!(service.list().is_empty());
    }

    #[test]
    fn remove_builtin_is_rejected() {
        let home = temp_home();
        let mut store = store_at(home.path());
        let service = test_service(home.path());
        service.startup(&mut store);

        let err = service
            .remove(&mut store, builtin::BUILTIN_PI_SOURCE)
            .unwrap_err();
        assert!(err.contains("不可移除"), "{err}");
        assert_eq!(service.list().len(), 1, "内置插件仍可用");
    }
}
