//! 密钥字段的三态：未设置 / 密钥链引用 / 明文。

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::secret::reference::{SCHEME, SecretRef};

/// 密钥字段的三态：值只有这三种形态，与它属于哪个字段无关。
///
/// - **未设置**（[`Unset`](SecretText::Unset)）：没有值。落盘为空串。
/// - **引用**（[`Ref`](SecretText::Ref)）：真值在系统密钥链里，字段只存
///   规范引用 URI。落盘为 `secret://…`。
/// - **明文**（[`Plaintext`](SecretText::Plaintext)）：真值直接写在字段里。
///   落盘为原字符串。
///
/// **允许不允许明文由字段自己决定，本类型不替字段做主**：`Plaintext` 是为
/// 「密文 / 明文二者皆可」的字段准备的。当前的 API Key 只允许「未设置」或
/// 「引用」——明文只会在读旧配置时出现，任何**新写入路径都必须先把它换成
/// 引用**（写密钥链 → 落引用）；这条约束落在 Provider 的写入路径上
/// （#145 的三态契约），而不是靠本类型拦住构造。
///
/// 脱敏是类型级隔离：[`Debug`](fmt::Debug) 只渲染「配没配」，绝不渲染值
/// ——调用点不必记得别打印，派生 `Debug` 的容器自动安全。
///
/// 取值只有三个入口：[`is_set`](SecretText::is_set)（配没配）、
/// [`reference`](SecretText::reference)（引用）与
/// [`plaintext`](SecretText::plaintext)（明文）。**没有**把引用 URI 当密钥
/// 值取出的方法——那会让调用方在「投影时忘了解析」的情况下，把引用 URI
/// 当成真值发给工具。
#[derive(Clone, PartialEq, Eq, Default)]
pub(crate) enum SecretText {
    /// 未设置：没有值。落盘为空串。
    #[default]
    Unset,
    /// 已设置：指向密钥链条目的规范引用。落盘为 `secret://…`。
    Ref(SecretRef),
    /// 已设置：真值直接写在字段里。落盘为原字符串。
    Plaintext(String),
}

impl SecretText {
    /// 未设置。
    pub(crate) fn unset() -> Self {
        Self::Unset
    }

    /// 已设置：指向密钥链条目的规范引用。
    ///
    /// API Key 这类「只允许密文」的字段，写路径经它落引用：真值先写进
    /// 密钥链，配置里只留这个引用（ADR 0002）。
    pub(crate) fn referenced(reference: SecretRef) -> Self {
        Self::Ref(reference)
    }

    /// 是否已设置：引用与明文都算已设置。
    pub(crate) fn is_set(&self) -> bool {
        !matches!(self, Self::Unset)
    }

    /// 规范引用；未设置或明文即 `None`。
    pub(crate) fn reference(&self) -> Option<&SecretRef> {
        match self {
            Self::Ref(reference) => Some(reference),
            _ => None,
        }
    }

    /// 明文；未设置或引用即 `None`。
    ///
    /// API Key 上它只会出现在读旧配置之后：迁移路径据此取真值，把明文写
    /// 进密钥链后，配置里换成引用（#147）。
    pub(crate) fn plaintext(&self) -> Option<&str> {
        match self {
            Self::Plaintext(value) => Some(value),
            _ => None,
        }
    }
}

impl fmt::Debug for SecretText {
    /// 只渲染「配没配」，绝不渲染值——明文也不带（ADR 0008：凭证绝不落盘
    /// 进日志）。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.is_set() { "<set>" } else { "<unset>" })
    }
}

impl Serialize for SecretText {
    /// 落盘形态：空串（未设置）/ 规范引用 URI（引用）/ 原字符串（明文）。
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Unset => serializer.serialize_str(""),
            Self::Ref(reference) => serializer.serialize_str(&reference.to_string()),
            Self::Plaintext(value) => serializer.serialize_str(value),
        }
    }
}

impl<'de> Deserialize<'de> for SecretText {
    /// 读入：空串即未设置；`secret://` 开头即引用（形态不合规即拒绝——
    /// 引用不合法不该被当成明文放行，ADR 0002）；其余即明文。
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "" => Self::Unset,
            text if text.starts_with(SCHEME) => {
                Self::Ref(SecretRef::parse(text).map_err(serde::de::Error::custom)?)
            }
            text => Self::Plaintext(text.to_owned()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本文件唯一的 canary 值：任何输出里都不该出现它（ADR 0008 的 L1 红线）。
    const CANARY: &str = "sk-canary-4b7e-do-not-leak";

    #[test]
    fn unset_serializes_as_empty_string() {
        assert_eq!(
            serde_json::to_value(SecretText::unset()).unwrap(),
            serde_json::json!("")
        );
    }

    #[test]
    fn reference_serializes_as_the_canonical_uri() {
        let reference = SecretRef::for_provider("openrouter").unwrap();

        assert_eq!(
            serde_json::to_value(SecretText::referenced(reference)).unwrap(),
            serde_json::json!(
                "secret://io.github.xezzon.agent-maestro/provider/openrouter/api_key"
            )
        );
    }

    #[test]
    fn plaintext_serializes_as_the_original_string() {
        let text = SecretText::Plaintext(CANARY.to_owned());

        assert_eq!(
            serde_json::to_value(&text).unwrap(),
            serde_json::json!(CANARY)
        );
    }

    /// 三态反序列化：空串 → 未设置，引用 → 引用，其余 → 明文。
    #[test]
    fn deserialization_reads_the_three_states() {
        let unset: SecretText = serde_json::from_str(r#""""#).unwrap();
        assert!(!unset.is_set());
        assert_eq!(unset.reference(), None);
        assert_eq!(unset.plaintext(), None);

        let reference: SecretText = serde_json::from_str(
            r#""secret://io.github.xezzon.agent-maestro/provider/openrouter/api_key""#,
        )
        .unwrap();
        assert!(reference.is_set());
        assert_eq!(
            reference.reference().map(SecretRef::slug),
            Some("openrouter")
        );
        assert_eq!(reference.plaintext(), None);

        let plaintext: SecretText = serde_json::from_str(r#""sk-legacy""#).unwrap();
        assert!(plaintext.is_set());
        assert_eq!(plaintext.reference(), None);
        assert_eq!(plaintext.plaintext(), Some("sk-legacy"));
    }

    /// `secret://` 开头即按引用处理：形态不合规一律拒绝，不当明文放行。
    #[test]
    fn malformed_reference_is_rejected_instead_of_read_as_plaintext() {
        let error = serde_json::from_str::<SecretText>(
            r#""secret://io.github.xezzon.agent-maestro/provider/openrouter""#,
        )
        .unwrap_err();

        assert!(
            error.to_string().contains("api_key"),
            "错误应说清形态不对：{error}"
        );
    }

    /// 三态 round-trip：序列化后再读回，形态不变。
    #[test]
    fn three_states_round_trip() {
        let cases = [
            SecretText::unset(),
            SecretText::referenced(SecretRef::for_provider("openrouter").unwrap()),
            SecretText::Plaintext(CANARY.to_owned()),
        ];

        for case in cases {
            let text = serde_json::to_string(&case).unwrap();

            assert_eq!(
                serde_json::from_str::<SecretText>(&text).unwrap(),
                case,
                "text={text}"
            );
        }
    }

    /// 三个取值口把三态分得开：未设置什么都不给、引用只给引用、明文只给明文。
    #[test]
    fn accessors_separate_the_three_states() {
        let unset = SecretText::unset();
        assert!(!unset.is_set());
        assert_eq!(unset.reference(), None);
        assert_eq!(unset.plaintext(), None);

        let referenced = SecretText::referenced(SecretRef::for_provider("openrouter").unwrap());
        assert!(referenced.is_set());
        assert_eq!(
            referenced.reference().map(SecretRef::slug),
            Some("openrouter")
        );
        assert_eq!(referenced.plaintext(), None);

        let plaintext = SecretText::Plaintext("sk-standalone".to_owned());
        assert!(plaintext.is_set());
        assert_eq!(plaintext.reference(), None);
        assert_eq!(plaintext.plaintext(), Some("sk-standalone"));
    }

    /// Debug 脱敏：三种形态都不带值（明文也不带）。
    #[test]
    fn debug_output_never_contains_the_plaintext() {
        let cases = [
            (SecretText::unset(), "<unset>"),
            (
                SecretText::referenced(SecretRef::for_provider("openrouter").unwrap()),
                "<set>",
            ),
            (SecretText::Plaintext(CANARY.to_owned()), "<set>"),
        ];

        for (case, expected) in cases {
            let rendered = format!("{case:?}");

            assert!(
                !rendered.contains(CANARY),
                "Debug 输出不得包含真值：{rendered}"
            );
            assert!(
                rendered.contains(expected),
                "渲染应为 {expected}：{rendered}"
            );
        }
    }
}
