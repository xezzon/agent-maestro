//! 占位符插值（ADR 0014 / ADR 0015）：把配置值里的占位符替换为变量的实际值。
//!
//! 语法遵循第三方库 [`subst`]（0.3.x）：`${name}` 与短式 `$name`、`${name:default}`
//! 内联默认值（变量名命中表时优先于内联默认值），反斜杠转义 `$ \ : { }`。
//! 插值在投影时由宿主执行，插件只收到替换后的字面值。
//!
//! 本模块是 ADR 0014 的换库接缝：`interpolate_value` 是唯一接触 subst 的纯函数，
//! 测试钉住其语义（升级或更换库改变行为即红，换库成本因此可控）；逐字段的
//! 结构知识在 `Provider::interpolate`，跨 Provider 的编排在 `interpolate_providers`。

use std::collections::BTreeMap;

use crate::provider::Provider;
use crate::store::Variables;

/// 对单个字符串值插值；失败原因即库报错（只含变量名与位置，不含值与密钥）。
pub(crate) fn interpolate_value(template: &str, variables: &Variables) -> Result<String, String> {
    subst::substitute(template, variables).map_err(|e| e.to_string())
}

/// 对传入的每个 Provider 插值，返回替换后的字面值副本。
///
/// 作用域＝Provider 中除 `api_key`、模型 `id` 与 header **键**外的字符串值
/// （ADR 0015；header 的值是配置值、参与插值）。
/// 是否参与投影由调用方决定：本函数不感知 `enabled`，对传入的每一项都插值；
/// 任一字段失败即整体失败，原因含 slug + 字段名 + 库报错，不含值与密钥。
pub fn interpolate_providers(
    providers: &BTreeMap<String, Provider>,
    variables: &Variables,
) -> Result<BTreeMap<String, Provider>, String> {
    let mut projected = BTreeMap::new();
    for (slug, provider) in providers {
        match provider.interpolate(variables) {
            Ok(provider) => {
                projected.insert(slug.clone(), provider);
            }
            Err((field, reason)) => {
                return Err(format!("Provider「{slug}」的 {field} 插值失败：{reason}"));
            }
        }
    }
    Ok(projected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Protocol;

    fn vars(entries: &[(&str, &str)]) -> Variables {
        entries
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    fn interpolated(template: &str, variables: &Variables) -> String {
        interpolate_value(template, variables).unwrap()
    }

    fn error(template: &str, variables: &Variables) -> String {
        interpolate_value(template, variables).unwrap_err()
    }

    /// 换库哨兵（ADR 0014）：`${name}` 长式替换，前后缀拼接原样保留。
    #[test]
    fn long_form_placeholder_with_prefix_and_suffix() {
        let variables = vars(&[("HOST", "api.example.com")]);
        assert_eq!(
            interpolated("https://${HOST}/v1", &variables),
            "https://api.example.com/v1"
        );
    }

    /// 换库哨兵：短式 `$name` 同样替换，名字读到非名字字符为止。
    #[test]
    fn short_form_placeholder_stops_at_non_name_character() {
        let variables = vars(&[("name", "world")]);
        assert_eq!(interpolated("Hello $name!", &variables), "Hello world!");
    }

    /// 换库哨兵：`${name:default}` 内联默认值，变量名命中表时优先于内联默认值。
    #[test]
    fn inline_default_only_applies_when_the_name_is_not_defined() {
        let defined = vars(&[("name", "table")]);
        assert_eq!(interpolated("${name:builtin}", &defined), "table");

        let missing = vars(&[]);
        assert_eq!(interpolated("${name:builtin}", &missing), "builtin");
    }

    /// 换库哨兵：内联默认值内部递归替换；变量值不递归（无链式解析，ADR 0015）。
    #[test]
    fn default_values_expand_recursively_but_variable_values_do_not() {
        let variables = vars(&[("home", "/home/u"), ("b", "b-value")]);
        assert_eq!(
            interpolated("${conf:$home/.config}", &variables),
            "/home/u/.config"
        );
        assert_eq!(interpolated("${a:$b}", &variables), "b-value");
        assert_eq!(
            interpolated("${a}", &vars(&[("a", "$b")])),
            "$b",
            "变量值里的占位符原样输出"
        );
    }

    /// 换库哨兵：反斜杠转义 `$ \ : { }`，转义后的字符原样保留。
    #[test]
    fn backslash_escapes_dollar_backslash_colon_and_braces() {
        let variables = vars(&[("name", "v")]);
        assert_eq!(interpolated(r"\$", &variables), "$");
        assert_eq!(interpolated(r"\\", &variables), "\\");
        assert_eq!(interpolated(r"\:", &variables), ":");
        assert_eq!(interpolated(r"\{", &variables), "{");
        assert_eq!(interpolated(r"\}", &variables), "}");
        // 转义后的 `$` 不再构成占位符：值里含字面 `$` 须这样书写。
        assert_eq!(interpolated(r"sk-\$abc${name}", &variables), "sk-$abcv");
    }

    /// 换库哨兵：未定义变量、未闭合 `${`、空名、非法转义、串尾悬空 `$` 一律显式失败。
    #[test]
    fn malformed_or_unresolved_values_fail_loudly() {
        let variables = vars(&[]);
        let missing = error("${MISSING}", &variables);
        assert!(missing.contains("MISSING"), "{missing}");
        assert!(error("https://${HOST/v1", &variables).contains("closing brace"));
        assert!(error("https://${}/v1", &variables).contains("variable name"));
        assert!(error(r"sk-\x", &variables).contains("escape"));
        // 串尾悬空 `$` 同样显式失败，不静默按字面量保留。
        let dangling = error("sk-$", &variables);
        assert!(!dangling.is_empty(), "{dangling}");
    }

    /// 换库哨兵：值本身以 `$`/`\` 开头不会被特殊化——含 `$` 的字面值必须转义，
    /// 未转义即报错（ADR 0014 接受的行为变化，约束写进 SDK README）。
    #[test]
    fn unescaped_dollar_in_a_value_is_treated_as_a_reference() {
        let variables = vars(&[]);
        let err = error("sk-$abc", &variables);
        assert!(err.contains("abc"), "{err}");
    }

    /// 任一字段失败即整体失败：原因含 slug + 字段名 + 变量名，不含值与密钥。
    #[test]
    fn provider_interpolation_failure_reports_slug_field_and_variable_only() {
        let mut providers = BTreeMap::new();
        providers.insert(
            "gateway".to_owned(),
            Provider {
                base_url: BTreeMap::from([(
                    Protocol::OpenaiCompletions,
                    "https://${HOST}/v1".to_owned(),
                )]),
                api_key: "sk-canary-do-not-leak".to_owned(),
                ..Default::default()
            },
        );

        let err = interpolate_providers(&providers, &vars(&[])).unwrap_err();

        assert!(err.contains("gateway"), "{err}");
        assert!(err.contains("base_url.openai-completions"), "{err}");
        assert!(err.contains("HOST"), "{err}");
        assert!(
            !err.contains("sk-canary-do-not-leak"),
            "失败原因不得携带值与密钥：{err}"
        );
    }
}
