//! 密钥引用：密钥链条目的规范标识（`secret://` URI）。
//!
//! 规范形态只有一种：
//! `secret://io.github.xezzon.agent-maestro/provider/<slug>/api_key`。
//!
//! 解析**逐字严格**：前缀、命名空间、account 形态、slug 字符集，任何一处
//! 偏差一律拒绝，不做宽容解析——宽容解析会让「拼错的引用」被当成合法引用，
//! 于是密钥取不到却看不出原因（ADR 0002）。

use std::fmt::{self, Display};

use crate::errors::SecretRefError;

/// 密钥链的命名空间（service）：固定值，所有条目共用。
pub(crate) const SERVICE: &str = "io.github.xezzon.agent-maestro";

/// 密钥引用的 URI 前缀：保留前缀，明文密钥不能以它开头。
pub(crate) const SCHEME: &str = "secret://";

/// account 的固定前缀：`provider/`。
const ACCOUNT_PREFIX: &str = "provider/";

/// account 的固定后缀：`/api_key`。
const ACCOUNT_SUFFIX: &str = "/api_key";

/// 密钥引用：指向密钥链条目的规范标识。
///
/// 引用与 slug 绑定：slug 不可变，引用因此稳定（GLOSSARY「密钥引用」）。
/// 相等性按规范形态判定——service 与 account 形态都是固定的，故等价于
/// 「slug 相同」。
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SecretRef {
    /// Provider 的 slug：引用里唯一可变的部分。
    slug: String,
}

impl SecretRef {
    /// 由 Provider 的 slug 构造规范引用。
    ///
    /// slug 不合法即拒绝：构造出的引用一定能被 [`SecretRef::parse`] 原样解析
    /// 回来——「引用解析不回来」这种状态不该存在。
    pub(crate) fn for_provider(slug: &str) -> Result<Self, SecretRefError> {
        if !is_valid_slug(slug) {
            return Err(SecretRefError::InvalidSlug);
        }
        Ok(Self {
            slug: slug.to_owned(),
        })
    }

    /// 严格解析：逐字比对，任何偏差一律拒绝。
    ///
    /// 拒绝的原因分四类（见 [`SecretRefError`]），便于错误文案说清「哪里
    /// 不对」而不是笼统地说「格式错误」。
    pub(crate) fn parse(text: &str) -> Result<Self, SecretRefError> {
        let rest = text
            .strip_prefix(SCHEME)
            .ok_or(SecretRefError::NotAReference)?;
        // 逐段剥掉固定字面量：命名空间、account 前缀、account 后缀。
        let rest = rest
            .strip_prefix(&format!("{SERVICE}/"))
            .ok_or(SecretRefError::WrongService)?;
        let rest = rest
            .strip_prefix(ACCOUNT_PREFIX)
            .ok_or(SecretRefError::WrongAccount)?;
        let slug = rest
            .strip_suffix(ACCOUNT_SUFFIX)
            .ok_or(SecretRefError::WrongAccount)?;
        if !is_valid_slug(slug) {
            return Err(SecretRefError::InvalidSlug);
        }
        Ok(Self {
            slug: slug.to_owned(),
        })
    }

    /// 这段文本是不是本 slug 的规范引用。
    ///
    /// 用于识别「前端回传自己刚拿到的引用」：逐字相同才算，别人的引用与
    /// 成形不成的 `secret://` 值一律不算（那不是本 Provider 的凭证）。
    pub(crate) fn is_canonical_for(slug: &str, text: &str) -> bool {
        match Self::for_provider(slug) {
            Ok(reference) => reference.to_string() == text,
            Err(_) => false,
        }
    }

    /// 引用反推出的 slug：供错误文案点名 Provider。
    ///
    /// slug 正是从 account 的中段反推出来的（`provider/<slug>/api_key`），
    /// 解析与构造走的是同一条规则。
    pub(crate) fn slug(&self) -> &str {
        &self.slug
    }

    /// 密钥链条目的 account：`provider/<slug>/api_key`。
    ///
    /// account 是密钥链里的条目名，与 service 一起定位一条目（GLOSSARY
    /// 「密钥链」）。
    pub(crate) fn account(&self) -> String {
        format!("{ACCOUNT_PREFIX}{}{ACCOUNT_SUFFIX}", self.slug)
    }

    /// 密钥链的 service：固定命名空间 [`SERVICE`]。
    pub(crate) fn service(&self) -> &'static str {
        SERVICE
    }
}

impl Display for SecretRef {
    /// 规范 URI：`secret://<service>/<account>`。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{SCHEME}{}/{}", self.service(), self.account())
    }
}

impl fmt::Debug for SecretRef {
    /// 渲染规范 URI：引用不含秘密，可随配置复制（GLOSSARY「密钥引用」），
    /// 故它可以原样进日志。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

/// Provider 的 slug 规则：小写字母开头，只含小写字母、数字、连字符与下划线。
///
/// 与界面的 slug 规则同一条（前端 `SLUG_PATTERN`）。两边各写一份——Rust 侧
/// 尚无统一的 slug 校验入口（Provider 的 slug 合法性目前由界面把关），此处
/// 照抄同一条规则，好让引用解析自己守住「只认规范形态」这条线。
fn is_valid_slug(slug: &str) -> bool {
    let mut chars = slug.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|char| {
        char.is_ascii_lowercase() || char.is_ascii_digit() || char == '-' || char == '_'
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 规范引用逐字钉住：前缀、命名空间、account 形态一个字符都不差。
    #[test]
    fn canonical_reference_is_verbatim() {
        let reference = SecretRef::for_provider("openrouter").unwrap();

        assert_eq!(
            reference.to_string(),
            "secret://io.github.xezzon.agent-maestro/provider/openrouter/api_key"
        );
        assert_eq!(reference.service(), "io.github.xezzon.agent-maestro");
        assert_eq!(reference.account(), "provider/openrouter/api_key");
    }

    /// 规范引用解析回来逐字不变（构造与解析互为逆运算）。
    #[test]
    fn canonical_reference_round_trips() {
        for slug in ["a", "openrouter", "acme-2", "acme_2", "a_b-c9"] {
            let reference = SecretRef::for_provider(slug).unwrap();

            let parsed = SecretRef::parse(&reference.to_string()).unwrap();

            assert_eq!(parsed, reference, "slug={slug}");
        }
    }

    /// 构造也守规范：slug 不合法即拒绝，故构造出的引用一定解析得回来。
    #[test]
    fn construction_rejects_an_invalid_slug() {
        for slug in ["", "OpenRouter", "2fast", "open router", "open/router"] {
            let error = SecretRef::for_provider(slug).unwrap_err();

            assert_eq!(error, SecretRefError::InvalidSlug, "slug={slug}");
        }
    }

    /// 严格解析：任何偏差一律拒绝，不做宽容解析。
    #[test]
    fn strict_parsing_rejects_every_deviation() {
        let cases = [
            // 前缀不对。
            ("", SecretRefError::NotAReference),
            ("provider/openrouter/api_key", SecretRefError::NotAReference),
            (
                "http://io.github.xezzon.agent-maestro/provider/openrouter/api_key",
                SecretRefError::NotAReference,
            ),
            (
                "SECRET://io.github.xezzon.agent-maestro/provider/openrouter/api_key",
                SecretRefError::NotAReference,
            ),
            // 命名空间不对。
            (
                "secret://io.github.xezzon.agent-maestro.evil/provider/openrouter/api_key",
                SecretRefError::WrongService,
            ),
            (
                "secret://openrouter/provider/openrouter/api_key",
                SecretRefError::WrongService,
            ),
            (
                "secret:///provider/openrouter/api_key",
                SecretRefError::WrongService,
            ),
            // account 形态不对。
            (
                "secret://io.github.xezzon.agent-maestro/provider/openrouter",
                SecretRefError::WrongAccount,
            ),
            (
                "secret://io.github.xezzon.agent-maestro/provider/openrouter/api_key/extra",
                SecretRefError::WrongAccount,
            ),
            (
                "secret://io.github.xezzon.agent-maestro/provider/openrouter/apikey",
                SecretRefError::WrongAccount,
            ),
            (
                "secret://io.github.xezzon.agent-maestro/openrouter/api_key",
                SecretRefError::WrongAccount,
            ),
            // slug 不合法。
            (
                "secret://io.github.xezzon.agent-maestro/provider//api_key",
                SecretRefError::InvalidSlug,
            ),
            (
                "secret://io.github.xezzon.agent-maestro/provider/OpenRouter/api_key",
                SecretRefError::InvalidSlug,
            ),
            (
                "secret://io.github.xezzon.agent-maestro/provider/2fast/api_key",
                SecretRefError::InvalidSlug,
            ),
            (
                "secret://io.github.xezzon.agent-maestro/provider/open router/api_key",
                SecretRefError::InvalidSlug,
            ),
        ];

        for (text, expected) in cases {
            let error = SecretRef::parse(text).unwrap_err();

            assert_eq!(error, expected, "text={text}");
        }
    }

    /// slug 反推：引用能点名 Provider（错误文案要用）。
    #[test]
    fn slug_is_recovered_from_the_reference() {
        let reference =
            SecretRef::parse("secret://io.github.xezzon.agent-maestro/provider/openrouter/api_key")
                .unwrap();

        assert_eq!(reference.slug(), "openrouter");
    }

    /// 相等性按规范形态判定：同一 slug 的两个引用相等。
    #[test]
    fn equality_follows_the_canonical_form() {
        let left = SecretRef::for_provider("openrouter").unwrap();
        let right =
            SecretRef::parse("secret://io.github.xezzon.agent-maestro/provider/openrouter/api_key")
                .unwrap();

        assert_eq!(left, right);
        assert_ne!(left, SecretRef::for_provider("other").unwrap());
    }
}
