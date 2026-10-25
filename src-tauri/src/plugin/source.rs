//! 插件来源与地址：来源字符串的解析与还原，以及 https 地址的判定。
//!
//! 来源是插件条目的唯一身份（见 docs/GLOSSARY.md「来源」）：内置于应用
//! （`builtin:<id>`）、一个指向 manifest.json 的 https URL，或一个指向
//! manifest.json 的本机绝对路径。来源字符串此前在四处各判一次，每次都重新
//! 解析同一串文本；判出来的又只有「是哪种」而没有「是哪一条」。本模块把
//! 「是什么来源、是哪一条」变成值的一部分：解析一次，之后只 match。
//!
//! https 地址的判定也在此处：来源 URL 与 manifest 的 `entry` 共用同一套规则
//! （只接受绝对 https URL，明文 http 与其它 scheme 一律拒绝）。

use std::path::{Path, PathBuf};

use crate::{errors::PluginError, plugin::builtin};

/// 插件来源：解析一次，随值传递。
///
/// 三种来源共用同一份 manifest 校验，`entry` 语义按来源分列，不做跨协议的
/// 统一解析（见 ADR 0006）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PluginSource {
    /// 内置于应用：载荷是插件 id（`builtin:<id>`），wasm 内嵌，`entry` 不参与解析。
    Builtin(String),
    /// https 来源：载荷是 manifest.json 的 URL，`entry` 必须是 https URL。
    Https(String),
    /// 本机指向 manifest.json 的绝对路径（本地调试回路）：`entry` 是 manifest
    /// 所在目录内的相对路径或 https URL。
    File(PathBuf),
}

impl PluginSource {
    /// 解析来源字符串；无法识别的形态返回 `None`。
    ///
    /// `builtin:<id>` 内置于应用；https URL 为正式发布来源；本机绝对路径为
    /// file 来源。其余形态（如旧配置残留的 Git 地址、相对路径）无法识别，
    /// 返回 `None` 由调用方报「暂不支持的插件来源」。
    pub(crate) fn parse(source: &str) -> Option<Self> {
        if let Some(id) = source.strip_prefix(builtin::SOURCE_PREFIX) {
            Some(Self::Builtin(id.to_owned()))
        } else if is_https_url(source) {
            Some(Self::Https(source.to_owned()))
        } else if Path::new(source).is_absolute() {
            Some(Self::File(PathBuf::from(source)))
        } else {
            None
        }
    }

    /// 解析来源字符串；无法识别的形态给出用户可见的原因。
    ///
    /// 文案只此一份：安装与重新加载此前各写一遍同样的句子，改一边就会与另一边
    /// 漂移。
    pub(crate) fn parse_or_err(source: &str) -> Result<Self, PluginError> {
        Self::parse(source).ok_or_else(|| PluginError::UnrecognizedSource {
            source: source.to_owned(),
        })
    }

    /// 来源字符串（条目的身份）：逐字还原成解析前的那一串。
    pub(crate) fn source(&self) -> String {
        match self {
            Self::Builtin(id) => builtin::source_of(id),
            Self::Https(url) => url.clone(),
            Self::File(path) => path.to_string_lossy().into_owned(),
        }
    }
}

/// 是否为绝对 https URL：明文 http、其他 scheme 与空主机一律拒绝。
pub(crate) fn is_https_url(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|parsed| parsed.scheme() == "https" && parsed.has_host())
}

/// 来源是否指向内置插件（`builtin:<id>` 前缀）。
///
/// 「是不是内置」此前在列表与移除两处各判一次，两处问的是同一句规则。
pub(crate) fn is_builtin_source(source: &str) -> bool {
    matches!(PluginSource::parse(source), Some(PluginSource::Builtin(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 来源解析一次即成为值：解析出来的载荷与来源字符串逐字对应。
    #[test]
    fn plugin_source_is_parsed_from_the_source_string() {
        assert_eq!(
            PluginSource::parse("builtin:pi"),
            Some(PluginSource::Builtin("pi".to_owned()))
        );
        assert_eq!(
            PluginSource::parse("https://example.com/manifest.json"),
            Some(PluginSource::Https(
                "https://example.com/manifest.json".to_owned()
            ))
        );
        assert_eq!(
            PluginSource::parse("/tmp/plugin/manifest.json"),
            Some(PluginSource::File(PathBuf::from(
                "/tmp/plugin/manifest.json"
            )))
        );
        assert_eq!(
            PluginSource::parse("plugins/pi/manifest.json"),
            None,
            "相对路径不是合法的来源身份（file 来源要求本机绝对路径）"
        );
        assert_eq!(PluginSource::parse("file:///tmp/manifest.json"), None);
        assert_eq!(PluginSource::parse("git://example.com/x.git"), None);
    }

    /// 来源字符串逐字还原：它是条目的身份，不能因为解析而变形。
    #[test]
    fn plugin_source_round_trips_to_the_original_string() {
        for source in [
            "builtin:pi",
            "https://example.com/manifest.json",
            "/tmp/plugin/manifest.json",
        ] {
            let parsed = PluginSource::parse(source).unwrap();
            assert_eq!(parsed.source(), source);
        }
    }

    #[test]
    fn is_https_url_accepts_only_absolute_https_urls() {
        assert!(is_https_url("https://example.com/manifest.json"));
        assert!(is_https_url("https://example.com:8443/a/b?c=d"));
        assert!(!is_https_url("http://example.com/manifest.json"));
        assert!(!is_https_url("file:///tmp/manifest.json"));
        assert!(!is_https_url("/tmp/manifest.json"));
        assert!(!is_https_url("https://"));
        assert!(!is_https_url(""));
    }
}
