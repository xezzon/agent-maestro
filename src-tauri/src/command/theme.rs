use super::log_outcome;

/// 跟随系统时，宿主是否要求应用把窗口主题钉在系统外观上。
///
/// Linux 的 GTK 实现把「窗口主题交还系统」当成一次亮色切换
/// （`gtk-application-prefer-dark-theme` 被置回 false）：交还之后原生标题栏是
/// 亮色，与系统的暗色外观对不上——界面是暗的、标题栏是亮的。此时须由应用自己
/// 把窗口主题钉在读到的系统外观上，标题栏才与界面同明暗。
///
/// 其余平台把「交还」实现为「窗口随系统明暗变化」，交还即可，宿主自己跟上系统
/// 的明暗变化（macOS 的窗口外观、Windows 的 DWM 主题皆如此）。
#[tauri::command]
pub(crate) fn host_requires_pinned_theme() -> bool {
    let pinned = cfg!(target_os = "linux");
    log_outcome("host_requires_pinned_theme", None, &Ok::<(), String>(()));
    pinned
}
