//! 用户偏好（ADR 0023）：应用级用户偏好的形状，以及从磁盘读入时的归一化。
//!
//! 偏好是独立于领域配置（Provider、变量、插件）的一份顶层 JSON 文档，存于
//! `~/.maestro/preferences.json`。它与领域配置是不同类的数据：领域配置是投影
//! 的数据源头，损坏不可容忍（ADR 0001「坏文件拒绝写入，绝不静默覆盖」）；偏好
//! 只关乎应用外观，读不出来时安全退回默认值即可，不应阻塞应用。
//!
//! 本模块只声明「偏好长什么样」与「从磁盘读入时如何归一化」，不含读写行为
//! ——读写归 [`store::PreferencesStore`]，路径归 [`crate::paths`]，错误类型归
//! [`crate::errors`]。
//!
//! **读入宽松、写入严格**：磁盘上的脏值（缺字段、非法主题值）兜底并写回修正；
//! 命令边界的载荷则由反序列化直接拒掉非法主题值（前端保证只发三态之一）。
//! 两者的不对称是刻意的：磁盘上的脏值不该让应用报错，而前端传错值不该被静默
//! 接受。

mod store;

#[cfg(test)]
mod tests;

pub(crate) use store::PreferencesStore;

use serde::{Deserialize, Serialize};

/// 偏好文件 schema 版本（ADR 0023），与领域配置的
/// [`crate::config::CONFIG_VERSION`] 各自独立演进。
pub(crate) const PREFERENCES_VERSION: u32 = 1;

/// 主题偏好：亮色、暗色、跟随系统。
///
/// 落盘取值即变体名的小写，故 `rename_all` 与 [`Theme::as_str`] 恒一致；取值
/// 只在 [`Theme::as_str`] 一处写着。序列化即落盘取值（`light` / `dark` /
/// `system`）；反序列化严格——未知取值直接失败，故命令边界能拒掉非法主题值
/// （写入严格）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Theme {
    /// 亮色：应用外观固定为亮色，不随系统变化。
    Light,
    /// 暗色：应用外观固定为暗色，不随系统变化。
    Dark,
    /// 跟随系统：应用外观随操作系统的明暗外观变化。
    System,
}

impl Theme {
    /// 落盘取值（与序列化结果一致）。
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Theme::Light => "light",
            Theme::Dark => "dark",
            Theme::System => "system",
        }
    }

    /// 解析落盘取值；未知取值返回 `None`（由调用方兜底）。
    pub(crate) fn parse(value: &str) -> Option<Self> {
        [Theme::Light, Theme::Dark, Theme::System]
            .into_iter()
            .find(|theme| theme.as_str() == value)
    }
}

impl Default for Theme {
    /// 缺省即跟随系统：不必用户选择，应用外观就与操作系统一致。
    fn default() -> Self {
        Theme::System
    }
}

/// 偏好文档（`~/.maestro/preferences.json` 的顶层文档，version 1 schema）。
///
/// 缺字段即容器的缺省值（`version` 为当前版本、`theme` 为跟随系统），故缺字段
/// 的旧偏好文件读入后行为与缺省一致。命令载荷用的就是这一形状：反序列化拒绝
/// 未知主题值（写入严格）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Preferences {
    /// 偏好文件 schema 版本；写入时由存储层强制为 [`PREFERENCES_VERSION`]。
    pub(crate) version: u32,
    /// 主题偏好。
    pub(crate) theme: Theme,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: PREFERENCES_VERSION,
            theme: Theme::System,
        }
    }
}

/// 磁盘读入用的宽松形状：字段与取值都宽松（读入宽松，见 ADR 0023）。
///
/// 与命令载荷（严格）分开：磁盘上可能缺字段、主题值可能写成别的字符串甚至别的
/// 类型——那些都要兜底成默认值，而不是让整份文档解析失败。字段取原始 JSON 值
/// 而非具体类型，缺字段与非预期类型因此各走各的兜底规则。
#[derive(Debug, Deserialize)]
pub(crate) struct PreferencesFile {
    /// 版本：缺字段或 `null` 即按缺省补齐；显式给出但不受支持时按「不碰该文件」
    /// 处理。
    #[serde(default)]
    version: Option<serde_json::Value>,
    /// 主题：缺字段或取值非法一律兜底为跟随系统。
    #[serde(default)]
    theme: Option<serde_json::Value>,
}

/// 读入磁盘后的归一化结果。
pub(crate) enum Loaded {
    /// 读得出来：给出归一化后的偏好，以及是否需要写回修正。
    Usable {
        /// 归一化后的偏好（内存中即此值）。
        preferences: Preferences,
        /// 读入内容与归一化结果有出入（缺字段 / 非法取值）：需写回修正。
        needs_rewrite: bool,
    },
    /// 版本不受支持：按默认值运行，且不读写该文件。
    UnsupportedVersion {
        /// 文件里读到的版本取值（原样带进日志，便于用户对照文件）。
        found: String,
        /// 取值是不是整数：整数版本号可能来自更新版本；非整数（`1.0`、字符串
        /// 等）是文件形状被改坏，报错文案据此区分，不说成「更新版本写出」。
        integer: bool,
    },
}

impl PreferencesFile {
    /// 归一化：缺字段补缺省、非法主题值兜底为跟随系统。
    ///
    /// 版本不受支持时返回 [`Loaded::UnsupportedVersion`]——整数版本号可能来自
    /// 更新的版本，不该被改写，调用方据此按默认值运行且不碰它。
    pub(crate) fn normalize(self) -> Loaded {
        // `null` 与字段缺失同义（显式写 null 等于没给），都按缺省补齐。
        let version_was_absent = matches!(&self.version, None | Some(serde_json::Value::Null));
        let version = match &self.version {
            None | Some(serde_json::Value::Null) => PREFERENCES_VERSION,
            Some(value) => match value.as_u64() {
                Some(found) if found == u64::from(PREFERENCES_VERSION) => PREFERENCES_VERSION,
                Some(_) => {
                    return Loaded::UnsupportedVersion {
                        found: describe(value),
                        integer: true,
                    };
                }
                None => {
                    return Loaded::UnsupportedVersion {
                        found: describe(value),
                        integer: false,
                    };
                }
            },
        };

        let (theme, theme_needs_rewrite) = match &self.theme {
            None => (Theme::System, true),
            Some(serde_json::Value::String(raw)) => match Theme::parse(raw) {
                Some(theme) => (theme, false),
                None => (Theme::System, true),
            },
            Some(_) => (Theme::System, true),
        };

        Loaded::Usable {
            preferences: Preferences { version, theme },
            needs_rewrite: version_was_absent || theme_needs_rewrite,
        }
    }
}

/// 版本取值的可读描述（带进日志，让用户能对照文件里的内容）。
fn describe(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}
