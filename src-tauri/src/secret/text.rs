//! 凭证字段的三态：未设置 / 密钥链引用 / 旧明文。

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::secret::reference::{SCHEME, SecretRef};

/// 凭证字段的三态（ADR 0002）。
///
/// - **未设置**（[`Unset`](SecretText::Unset)）：没有凭证。落盘为空串。
/// - **引用**（[`Ref`](SecretText::Ref)）：真值在系统密钥链里，字段只存
///   规范引用 URI。落盘为 `secret://…`。
/// - **旧明文**（[`Plaintext`](SecretText::Plaintext)）：迁移前的旧配置里
///   直接写着真值。它**只可能来自旧配置的反序列化**——新写入路径一律产出
///   引用（写密钥链 → 落引用），不产出明文。
///
/// 脱敏是类型级隔离：[`Debug`](fmt::Debug) 只渲染「配没配」，绝不渲染值
/// ——调用点不必记得别打印，派生 `Debug` 的容器自动安全。
///
/// 取值只有三个入口：[`is_set`](SecretText::is_set)（配没配）、
/// [`reference`](SecretText::reference)（引用）与
/// [`plaintext`](SecretText::plaintext)（旧明文）。**没有**把引用 URI 当密钥
/// 值取出的方法——那会让调用方在「投影时忘了解析」的情况下，把引用 URI
/// 当成真值发给工具。
#[derive(Clone, PartialEq, Eq, Default)]
pub(crate) enum SecretText {
    /// 未设置：没有凭证。落盘为空串。
    #[default]
    Unset,
    /// 已设置：指向密钥链条目的规范引用。落盘为 `secret://…`。
    Ref(SecretRef),
    /// 旧明文：只可能来自旧配置的反序列化。落盘为原字符串。
    ///
    /// 载荷是 [`LegacyPlaintext`] 而非 `String`：它的构造入口只开在本模块
    /// 内，于是**新写入路径造不出旧明文**——写路径要用明文，只能经反序列
    /// 化从旧配置里读进来（迁移路径，ADR 0002）。
    Plaintext(LegacyPlaintext),
}

/// 旧配置里的明文凭证。
///
/// 字段私有：构造入口只开在定义它的模块内，故 crate 其余部分拿不到它，
/// 也就造不出 [`SecretText::Plaintext`]。取值走
/// [`SecretText::plaintext`]。
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct LegacyPlaintext(String);

impl LegacyPlaintext {
    /// 明文本身。
    fn as_str(&self) -> &str {
        &self.0
    }
}

impl SecretText {
    /// 未设置。
    pub(crate) fn unset() -> Self {
        Self::Unset
    }

    /// 已设置：指向密钥链条目的规范引用。
    ///
    /// 这是新写入路径唯一的「已设置」构造方式——真值先写进密钥链，配置里
    /// 只留这个引用（ADR 0002）。
    pub(crate) fn referenced(reference: SecretRef) -> Self {
        Self::Ref(reference)
    }

    /// 是否已设置：引用与旧明文都算已设置。
    pub(crate) fn is_set(&self) -> bool {
        !matches!(self, Self::Unset)
    }

    /// 规范引用；未设置或旧明文即 `None`。
    pub(crate) fn reference(&self) -> Option<&SecretRef> {
        match self {
            Self::Ref(reference) => Some(reference),
            _ => None,
        }
    }

    /// 旧明文；未设置或引用即 `None`。
    ///
    /// 只有迁移路径用它取真值：把旧明文写进密钥链后，配置里换成引用。
    pub(crate) fn plaintext(&self) -> Option<&str> {
        match self {
            Self::Plaintext(value) => Some(value.as_str()),
            _ => None,
        }
    }
}

impl fmt::Debug for SecretText {
    /// 只渲染「配没配」，绝不渲染值——旧明文也不带（ADR 0008：凭证绝不落盘
    /// 进日志）。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.is_set() { "<set>" } else { "<unset>" })
    }
}

impl Serialize for SecretText {
    /// 落盘形态：空串（未设置）/ 规范引用 URI（已设置）/ 原字符串（旧明文）。
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Unset => serializer.serialize_str(""),
            Self::Ref(reference) => serializer.serialize_str(&reference.to_string()),
            Self::Plaintext(value) => serializer.serialize_str(value.as_str()),
        }
    }
}

impl<'de> Deserialize<'de> for SecretText {
    /// 读入：空串即未设置；`secret://` 开头即引用（形态不合规即拒绝——
    /// 引用不合法不该被当成明文放行，ADR 0002）；其余即旧明文。
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "" => Self::Unset,
            text if text.starts_with(SCHEME) => {
                Self::Ref(SecretRef::parse(text).map_err(serde::de::Error::custom)?)
            }
            text => Self::Plaintext(LegacyPlaintext(text.to_owned())),
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
        let text = SecretText::Plaintext(LegacyPlaintext(CANARY.to_owned()));

        assert_eq!(
            serde_json::to_value(&text).unwrap(),
            serde_json::json!(CANARY)
        );
    }

    /// 三态反序列化：空串 → 未设置，引用 → 引用，其余 → 旧明文。
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
            SecretText::Plaintext(LegacyPlaintext(CANARY.to_owned())),
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

    /// 新写入路径的两个构造入口都不产出旧明文：只有反序列化能造出它。
    #[test]
    fn write_paths_never_produce_plaintext() {
        let unset = SecretText::unset();
        let referenced = SecretText::referenced(SecretRef::for_provider("openrouter").unwrap());

        assert_eq!(unset.plaintext(), None);
        assert_eq!(referenced.plaintext(), None);
        assert_eq!(unset.reference(), None);
        assert!(referenced.reference().is_some());
    }

    /// Debug 脱敏：三种形态都不带值（旧明文也不带）。
    #[test]
    fn debug_output_never_contains_the_plaintext() {
        let cases = [
            (SecretText::unset(), "<unset>"),
            (
                SecretText::referenced(SecretRef::for_provider("openrouter").unwrap()),
                "<set>",
            ),
            (
                SecretText::Plaintext(LegacyPlaintext(CANARY.to_owned())),
                "<set>",
            ),
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
