use std::collections::BTreeMap;

use tauri::{AppHandle, Manager, State};

use super::log_outcome;
use crate::{
    AppStore,
    plugin::{PluginApplyReport, PluginConfig, PluginService, PluginView},
    provider::Provider,
};

/// 列出插件注册表（metadata 与加载状态）。
/// store 处于保护状态时报错，而非静默返回空列表误导用户；
/// 该检查由 `service.list` 内部的短锁完成，此处不得再持 store 锁——
/// 否则与 `list` 内部加锁构成同线程重入，死锁。
#[tauri::command]
pub(crate) fn list_plugins(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
) -> Result<Vec<PluginView>, String> {
    let outcome = service.list(&store);
    log_outcome("list_plugins", "", &outcome);
    outcome
}

/// 启用/禁用插件。
#[tauri::command]
pub(crate) fn set_plugin_enabled(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
    source: String,
    enabled: bool,
) -> Result<(), String> {
    let outcome = service.set_enabled(&store, &source, enabled);
    log_outcome(
        "set_plugin_enabled",
        &format!("source={source} enabled={enabled}"),
        &outcome,
    );
    outcome
}

/// 添加插件（来源为指向 manifest.json 的 https 地址或本机绝对路径）：获取 manifest 与
/// wasm、校验并落位，成功后才写入条目。
///
/// 失败不写条目、不落位；清理残留失败时会把残留路径一并返回界面。
/// 修复后重新添加即可（见 ADR 0006）。
#[tauri::command]
pub(crate) async fn add_plugin(app: AppHandle, source: String) -> Result<(), String> {
    // 安装类命令另加进入行：「有进入、无结果」正是定位卡住的证据（如 #46）。
    log::info!("add_plugin start: source={source}");
    // 识别参数在闭包 move 前格式化；payload 本身不进日志。
    let context = format!("source={source}");
    let outcome = on_install_pool(app, move |store, service| {
        service.add_plugin(store, &source)
    })
    .await;
    log_outcome("add_plugin", &context, &outcome);
    outcome
}

/// 重新加载插件：按配置中的来源重新获取 manifest 与 wasm，按「先 manifest、
/// 后 wasm」逐文件覆盖落位产物（写失败可能留下混合态，见 ADR 0018）。
#[tauri::command]
pub(crate) async fn reload_plugin(app: AppHandle, source: String) -> Result<(), String> {
    log::info!("reload_plugin start: source={source}");
    let context = format!("source={source}");
    let outcome = on_install_pool(app, move |store, service| {
        service.reload_plugin(store, &source)
    })
    .await;
    log_outcome("reload_plugin", &context, &outcome);
    outcome
}

/// 移除插件：删配置条目与落位目录（幂等），不联网。
#[tauri::command]
pub(crate) fn remove_plugin(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
    source: String,
) -> Result<(), String> {
    let outcome = service.remove_plugin(&store, &source);
    log_outcome("remove_plugin", &format!("source={source}"), &outcome);
    outcome
}

/// 读取插件配置（ADR 0018）：该插件对全局变量的覆盖与私有表单数据。
/// 配置不存在返回空配置（新装插件开箱即用），损坏则报错。
#[tauri::command]
pub(crate) fn get_plugin_config(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
    source: String,
) -> Result<PluginConfig, String> {
    let outcome = service.get_config(&store, &source);
    log_outcome("get_plugin_config", &format!("source={source}"), &outcome);
    outcome
}

/// 写入插件配置（整包替换，ADR 0018）：与安装/重载/移除共用安装锁互斥，
/// 写路径可能在临界区等待含网络下载的安装类命令，故走阻塞线程池执行。
/// 识别参数（source）进日志；变量值与表单内容不进日志（ADR 0008）。
#[tauri::command]
pub(crate) async fn set_plugin_config(
    app: AppHandle,
    source: String,
    config: PluginConfig,
) -> Result<(), String> {
    // 安装类命令另加进入行：「有进入、无结果」正是定位卡住的证据（如 #46）。
    log::info!("set_plugin_config start: source={source}");
    // 识别参数在闭包 move 前格式化；payload 本身不进日志。
    let context = format!("source={source}");
    let outcome = on_install_pool(app, move |store, service| {
        service.set_config(store, &source, &config)
    })
    .await;
    log_outcome("set_plugin_config", &context, &outcome);
    outcome
}

/// 挑出参与投影的 Provider（#94）：`enabled == false` 者整体不参与——既不写入
/// 插件，也不做占位符插值，因此其字段里的占位符错误不会牵连整个插件失败。
///
/// 在读取配置的临界区内调用：返回值即脱离 store 锁的那份自有快照（筛选与快照
/// 合并为一次克隆，且只克隆启用项）。
fn enabled_providers(providers: &BTreeMap<String, Provider>) -> BTreeMap<String, Provider> {
    providers
        .iter()
        .filter(|(_, provider)| provider.enabled)
        .map(|(slug, provider)| (slug.clone(), provider.clone()))
        .collect()
}

/// 应用到工具：调用所有已启用且加载成功的插件执行投影，
/// 返回逐插件结果（写入的文件、跳过的 Provider、失败原因）。
///
/// 禁用 Provider（`enabled == false`）在读取配置时即剔除（#94），不进入插值
/// 与写入。store 锁在读取 providers 后即释放，注册表锁也仅用于快照：
/// 插件执行时长不受应用控制，执行全程不持锁，不得阻塞其它命令。
#[tauri::command]
pub(crate) fn apply_providers(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
) -> Result<Vec<PluginApplyReport>, String> {
    // `?` 在闭包内传播：读 store 的错误也被捕获进 outcome，与 write_providers 的错误
    // 统一走「outcome → 日志 → 返回」路径。若平铺到函数体，`?` 会提前返回并跳过失败日志。
    let outcome = (|| -> Result<Vec<PluginApplyReport>, String> {
        let (providers, variables) = {
            let guard = store.read()?;
            let config = guard.get()?;
            (
                enabled_providers(&config.providers),
                config.variables.clone(),
            )
        };
        service.write_providers(&providers, &variables)
    })();
    // 结果行携带逐插件状态摘要（id:status），命令 payload 不进日志。
    match &outcome {
        Ok(reports) => {
            let summary = reports
                .iter()
                .map(|report| format!("{}:{}", report.id, report.status))
                .collect::<Vec<_>>()
                .join(" ");
            log::info!("apply_providers ok: {summary}");
        }
        Err(reason) => log::error!("apply_providers failed: {reason}"),
    }
    outcome
}

/// 在阻塞线程池执行含网络下载的安装类操作：下载可能持续数秒，
/// 不得占用 IPC 线程。状态在阻塞任务内获取，避免跨线程持有引用。
///
/// 配置存储的锁由插件服务按短临界区自行获取：这里绝不代为持锁，
/// 下载、校验与落位全程不阻塞其它命令。
async fn on_install_pool<T, F>(app: AppHandle, task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&AppStore, &PluginService) -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let store = app.state::<AppStore>();
        let service = app.state::<PluginService>();
        task(&store, &service)
    })
    .await
    .map_err(|e| format!("插件安装任务执行失败：{e}"))?
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs};

    use super::enabled_providers;
    use crate::{
        plugin::testutil::{store_at, temp_home, test_service},
        provider::{Protocol, Provider},
    };

    /// 禁用 Provider 不参与投影（#94）：先投影一次留下条目，再禁用并重新投影，
    /// 该条目连同其 api_key 一并从 models.json 消失——覆盖「禁用前已投影、
    /// 禁用后移除」的行为。
    #[test]
    fn enabled_providers_removes_a_formerly_projected_provider_once_disabled() {
        let home = temp_home();
        let store = store_at(home.path());
        let service = test_service(home.path());
        service.startup(&store).unwrap();

        let mut providers = BTreeMap::from([
            (
                "gateway".to_owned(),
                Provider {
                    base_url: BTreeMap::from([(
                        Protocol::OpenaiCompletions,
                        "https://api.example.com/v1".to_owned(),
                    )]),
                    ..Provider::default()
                },
            ),
            (
                "offline".to_owned(),
                Provider {
                    base_url: BTreeMap::from([(
                        Protocol::OpenaiCompletions,
                        "https://offline.example.com/v1".to_owned(),
                    )]),
                    api_key: "sk-disabled-must-not-leak".to_owned(),
                    ..Provider::default()
                },
            ),
        ]);

        let models_path = home.path().join(".pi").join("agent").join("models.json");
        let first = service
            .write_providers(&enabled_providers(&providers), &BTreeMap::new())
            .unwrap();
        assert_eq!(first[0].status, "applied", "{:?}", first[0].reason);
        let written: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&models_path).unwrap()).unwrap();
        assert_eq!(
            written["providers"]["offline"]["baseUrl"], "https://offline.example.com/v1",
            "禁用前该条目应已投影"
        );

        providers.get_mut("offline").unwrap().enabled = false;
        let second = service
            .write_providers(&enabled_providers(&providers), &BTreeMap::new())
            .unwrap();
        assert_eq!(second[0].status, "applied", "{:?}", second[0].reason);
        let raw = fs::read_to_string(&models_path).unwrap();
        let written: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            written["providers"]["gateway"]["baseUrl"], "https://api.example.com/v1",
            "启用项照常投影"
        );
        assert!(
            written["providers"].get("offline").is_none(),
            "禁用后该条目应从投影中移除：{written}"
        );
        assert!(!raw.contains("sk-disabled-must-not-leak"));
    }
}
