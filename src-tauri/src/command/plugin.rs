use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use super::log_outcome;
use crate::{
    AppStore,
    plugin::{
        PluginConfig, PluginOutcome, PluginRecord, PluginService, ProjectionStatus,
        SkippedProvider, apply_summary,
    },
};

/// 插件列表视图（命令返回给前端的形态）：**线格式契约**。
///
/// 插件注册表的形状（`config_dir` / `settings_schema` / `error`）本来就是给
/// 界面看的，硬塞回领域反而更假——故它住在交付层，与 `ProviderRequest` 同层。
/// 服务层只给内部形状（[`PluginRecord`]），映射在下方（#141 待决策 A）。
#[derive(Debug, Serialize)]
pub(crate) struct PluginView {
    source: String,
    /// 来源以内置前缀标识；内置插件可禁用、不可移除。
    builtin: bool,
    enabled: bool,
    id: String,
    /// manifest 声明并解析出的配置目录（宿主绝对路径）。
    config_dir: Option<String>,
    /// manifest 内联的 settings_schema（ADR 0017），透传给前端渲染表单；
    /// 缺省即该插件无需用户填写。
    settings_schema: Option<serde_json::Value>,
    error: Option<String>,
}

impl From<PluginRecord> for PluginView {
    fn from(record: PluginRecord) -> Self {
        Self {
            source: record.source,
            builtin: record.builtin,
            enabled: record.enabled,
            id: record.id,
            config_dir: record.config_dir.map(|dir| dir.display().to_string()),
            settings_schema: record.settings_schema,
            error: record.error,
        }
    }
}

/// 逐插件投影的状态（线格式）：已写入 / 失败 / 未执行。
///
/// 合法值由类型固定，线格式为小写词（`applied` / `failed` / `skipped`），
/// 前端按同一组词渲染标签。序列化形态与线格式由本模块的测试钉住：改词即
/// 前端标签静默失效。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ApplyStatus {
    /// 已写入：插件执行成功并写出了文件。
    Applied,
    /// 失败：插件执行了但返回错误（不写任何文件）。
    Failed,
    /// 未执行投影：插件已禁用或装载失败。
    Skipped,
}

impl From<ProjectionStatus> for ApplyStatus {
    fn from(status: ProjectionStatus) -> Self {
        match status {
            ProjectionStatus::Applied => ApplyStatus::Applied,
            ProjectionStatus::Failed => ApplyStatus::Failed,
            ProjectionStatus::Skipped => ApplyStatus::Skipped,
        }
    }
}

/// 逐插件投影报告（线格式）：状态、已写入文件、跳过的 Provider、失败原因。
#[derive(Debug, Serialize)]
pub(crate) struct PluginApplyReport {
    pub(crate) id: String,
    /// 投影状态（`applied` / `failed` / `skipped`）。
    pub(crate) status: ApplyStatus,
    /// 已写入文件的宿主绝对路径列表（插件返回相对 config_dir 的路径，由宿主拼接）。
    files: Vec<String>,
    skipped: Vec<SkippedProvider>,
    pub(crate) reason: Option<String>,
}

impl From<PluginOutcome> for PluginApplyReport {
    fn from(outcome: PluginOutcome) -> Self {
        Self {
            id: outcome.id,
            status: outcome.status.into(),
            files: outcome.files,
            skipped: outcome.skipped,
            reason: outcome.reason,
        }
    }
}

/// 列出插件注册表（metadata 与加载状态）。
/// store 处于保护状态时报错，而非静默返回空列表误导用户；
/// 该检查由 `service.list` 内部的短锁完成，此处不得再持 store 锁——
/// 否则与 `list` 内部加锁构成同线程重入，死锁。
#[tauri::command]
pub(crate) fn list_plugins(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
) -> Result<Vec<PluginView>, String> {
    let outcome = service.list(store.inner()).map(|records| {
        records
            .into_iter()
            .map(PluginView::from)
            .collect::<Vec<_>>()
    });
    log_outcome("list_plugins", None, &outcome);
    outcome
}

/// 启用/禁用插件。
///
/// 前端送来的是一个开关；服务层按意图分成 `enable` / `disable` 两个方法
/// （各自装载或降级注册表，不靠布尔分叉），命令层只做分派。
#[tauri::command]
pub(crate) fn set_plugin_enabled(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
    source: String,
    enabled: bool,
) -> Result<(), String> {
    let outcome = if enabled {
        service.enable(store.inner(), &source)
    } else {
        service.disable(store.inner(), &source)
    };
    log_outcome(
        "set_plugin_enabled",
        Some(&format!("source={source} enabled={enabled}")),
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
    log_outcome("add_plugin", Some(&context), &outcome);
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
    log_outcome("reload_plugin", Some(&context), &outcome);
    outcome
}

/// 移除插件：删配置条目与落位目录（幂等），不联网。
#[tauri::command]
pub(crate) fn remove_plugin(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
    source: String,
) -> Result<(), String> {
    let outcome = service.remove_plugin(store.inner(), &source);
    log_outcome("remove_plugin", Some(&format!("source={source}")), &outcome);
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
    let outcome = service.get_config(store.inner(), &source);
    log_outcome(
        "get_plugin_config",
        Some(&format!("source={source}")),
        &outcome,
    );
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
    log_outcome("set_plugin_config", Some(&context), &outcome);
    outcome
}

/// 应用到工具：调用所有已启用且加载成功的插件执行投影，
/// 返回逐插件结果（写入的文件、跳过的 Provider、失败原因）。
///
/// 命令层只做分派：读配置、挑启用项与执行投影都在插件服务里（见
/// `PluginService::apply_providers`）。结果行按 ADR 0008 携带逐插件状态
/// 摘要——某个插件失败时，日志里要能看出是哪一个；没有插件时不带上下文，
/// 成功行因此不留尾随的冒号与空格。
#[tauri::command]
pub(crate) fn apply_providers(
    store: State<'_, AppStore>,
    service: State<'_, PluginService>,
) -> Result<Vec<PluginApplyReport>, String> {
    let outcomes = service.apply_providers(store.inner());
    // 摘要取自服务层的内部形状：状态词由服务层产出（ADR 0008 的日志词），
    // 与线格式同形（由 `apply_status_wire_values_are_pinned` 钉住）。
    let context = outcomes
        .as_ref()
        .ok()
        .and_then(|outcomes| apply_summary(outcomes));
    let outcome = outcomes.map(|outcomes| {
        outcomes
            .into_iter()
            .map(PluginApplyReport::from)
            .collect::<Vec<_>>()
    });
    log_outcome("apply_providers", context.as_deref(), &outcome);
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
    use std::path::PathBuf;

    use super::{ApplyStatus, PluginApplyReport, PluginView};
    use crate::plugin::{PluginOutcome, PluginRecord, ProjectionStatus};

    /// 投影状态的线格式钉死：序列化出来就是 `applied` / `failed` / `skipped`
    /// 三个小写词。前端按同一组词渲染标签（见
    /// `src/pages/provider/applyStatus.js`），改词即前端标签静默失效——
    /// 故这三条断言是跨端契约的护栏。
    ///
    /// 同一组词也是服务层日志行用的词（ADR 0008）：两处同形，日志与界面才
    /// 不会各说各话。
    #[test]
    fn apply_status_wire_values_are_pinned() {
        for (status, wire, internal) in [
            (ApplyStatus::Applied, "applied", ProjectionStatus::Applied),
            (ApplyStatus::Failed, "failed", ProjectionStatus::Failed),
            (ApplyStatus::Skipped, "skipped", ProjectionStatus::Skipped),
        ] {
            assert_eq!(
                serde_json::to_value(status).unwrap(),
                serde_json::json!(wire),
                "序列化形态与线格式词一致"
            );
            assert_eq!(internal.to_string(), wire, "日志词与线格式词同形");
        }
    }

    /// 映射：服务层的注册表描述 → 线格式。配置目录在交付层才转成字符串——
    /// 服务层给的是路径，路径怎么显示给界面是交付层的事。
    #[test]
    fn plugin_view_maps_the_internal_record_to_the_wire_shape() {
        let record = PluginRecord {
            source: "builtin:pi".to_owned(),
            builtin: true,
            enabled: true,
            id: "pi".to_owned(),
            config_dir: Some(PathBuf::from("/home/u/.pi")),
            settings_schema: None,
            error: None,
        };

        let view = PluginView::from(record);

        assert_eq!(view.id, "pi");
        assert!(view.builtin);
        assert_eq!(view.config_dir.as_deref(), Some("/home/u/.pi"));
    }

    /// 映射：投影结果 → 线格式（状态词逐字对应）。
    #[test]
    fn plugin_apply_report_maps_the_internal_outcome_to_the_wire_shape() {
        let outcome = PluginOutcome {
            id: "pi".to_owned(),
            status: ProjectionStatus::Failed,
            files: Vec::new(),
            skipped: Vec::new(),
            reason: Some("原因".to_owned()),
        };

        let report = PluginApplyReport::from(outcome);

        assert_eq!(report.status, ApplyStatus::Failed);
        assert_eq!(report.reason.as_deref(), Some("原因"));
    }
}
