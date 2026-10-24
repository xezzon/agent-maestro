//! 面向用户的错误文案（展示层）：把结构化的错误排版成一句话。
//!
//! 存储层（`store`）只产出结构化错误（[`StoreError`]），领域模块（`interpolate`、
//! `provider`）只产出结构化原因（`FieldError`）；句式由本模块一处产出——改文案、
//! 抽 i18n 或改为结构化输出（前端自己排版），都不必进存储层与领域层（#141 I-3）。
//!
//! 消费方有二：命令层（IPC 错误通道只收字符串，在此归一出口）与插件服务（透传
//! 存储错误时同样经此排版——`PluginView.error` 与投影报告的 `reason` 在服务层
//! 成形，同因必须同报）。因此本模块住在 crate 根而非 `command/` 下：插件服务
//! 不能反向依赖命令层，否则 command ↔ plugin 成环。

use crate::store::{CONFIG_VERSION, STORE_LOCK_POISONED, StoreError};

/// 把存储错误排版为面向最终用户的一句话；命令层归一出口。
///
/// 与投影时拦截（`crate::provider::interpolate_providers` 的调用方）共用插值
/// 失败句式（[`interpolation_failure_message`]）：同因同报、逐字一致。
pub(crate) fn user_message(err: StoreError) -> String {
    match err {
        StoreError::Corrupt { path, detail } => format!(
            "配置文件已损坏：{}\n原因：{detail}\n请修复或删除该文件后重启应用；在此之前 Maestro 拒绝任何写入，绝不会静默重建。",
            path.display()
        ),
        StoreError::UnsupportedVersion { path, found } => format!(
            "配置文件版本不受支持：{}\n文件中的 version 为 {found}，当前应用仅支持 {CONFIG_VERSION}。\n请修复该文件后重启应用；在此之前 Maestro 拒绝任何写入，绝不会静默重建。",
            path.display()
        ),
        StoreError::Io { path, detail } => {
            format!("读写配置文件失败：{}\n原因：{detail}", path.display())
        }
        StoreError::Serialize { detail } => {
            format!("序列化配置文件失败\n原因：{detail}")
        }
        StoreError::DuplicateSlug { slug } => format!("已存在同名 Provider：{slug}"),
        StoreError::MissingSlug { slug } => format!("Provider 不存在：{slug}"),
        StoreError::MissingSource { source } => format!("插件条目不存在：{source}"),
        StoreError::DuplicateSource { source } => {
            format!("已存在同一来源的插件条目：{source}")
        }
        StoreError::DuplicateId { id, source } => {
            format!("插件 id「{id}」已被来源 {source} 占用")
        }
        StoreError::Interpolation {
            slug,
            field,
            reason,
        } => interpolation_failure_message(&slug, &field, &reason),
        StoreError::InvalidVariableName { name } => format!(
            "变量名不合法：「{name}」\n变量名只能包含字母、数字与下划线（见 ADR 0014）。"
        ),
        StoreError::Unavailable => STORE_LOCK_POISONED.to_owned(),
    }
}

/// 插值失败的面向用户文案：slug + 字段名 + 原因。
///
/// 落盘前拦截（`StoreError::Interpolation`）与投影时拦截（`provider::
/// interpolate_providers` 的调用方）共用这一句式：同因同报、逐字一致，
/// 改一处即改两处。
pub(crate) fn interpolation_failure_message(slug: &str, field: &str, reason: &str) -> String {
    format!("Provider「{slug}」的 {field} 插值失败：{reason}")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// 共享文案：slug + 字段名 + 原因逐字入句式（落盘前拦截与投影时拦截共用）。
    /// （迁移自 interpolate.rs，文案逐字不变。）
    #[test]
    fn interpolation_failure_message_renders_slug_field_and_reason() {
        assert_eq!(
            interpolation_failure_message(
                "gateway",
                "base_url.openai-completions",
                "No such variable: $HOST"
            ),
            "Provider「gateway」的 base_url.openai-completions 插值失败：No such variable: $HOST"
        );
    }

    /// 落盘前拦截的文案逐字断言：slug、字段名、原因三者都在句子里，且与投影时
    /// 拦截同一句式（ADR 0015「报错与投影时同因同报」；共享构造函数见 issue #128）。
    /// （迁移自 store/tests.rs，文案逐字不变。）
    #[test]
    fn interpolation_store_error_message_is_verbatim() {
        let err = StoreError::Interpolation {
            slug: "gateway".to_owned(),
            field: "base_url.openai-completions".to_owned(),
            reason: "No such variable: $HOST".to_owned(),
        };

        assert_eq!(
            user_message(err),
            "Provider「gateway」的 base_url.openai-completions 插值失败：No such variable: $HOST"
        );
    }

    /// 保护态文案逐字钉住（迁移自 store/schema.rs 的 From 实现，逐字不变）：
    /// 路径、原因与修复指引三段排版都在。
    #[test]
    fn corrupt_error_message_is_verbatim() {
        let err = StoreError::Corrupt {
            path: PathBuf::from("/home/u/.maestro/config.json"),
            detail: "expected value at line 1 column 1".to_owned(),
        };

        assert_eq!(
            user_message(err),
            "配置文件已损坏：/home/u/.maestro/config.json\n原因：expected value at line 1 column 1\n请修复或删除该文件后重启应用；在此之前 Maestro 拒绝任何写入，绝不会静默重建。"
        );
    }

    #[test]
    fn unsupported_version_error_message_is_verbatim() {
        let err = StoreError::UnsupportedVersion {
            path: PathBuf::from("/home/u/.maestro/config.json"),
            found: 99,
        };

        assert_eq!(
            user_message(err),
            "配置文件版本不受支持：/home/u/.maestro/config.json\n文件中的 version 为 99，当前应用仅支持 1。\n请修复该文件后重启应用；在此之前 Maestro 拒绝任何写入，绝不会静默重建。"
        );
    }

    #[test]
    fn io_error_message_is_verbatim() {
        let err = StoreError::Io {
            path: PathBuf::from("/home/u/.maestro/config.json"),
            detail: "Permission denied (os error 13)".to_owned(),
        };

        assert_eq!(
            user_message(err),
            "读写配置文件失败：/home/u/.maestro/config.json\n原因：Permission denied (os error 13)"
        );
    }

    #[test]
    fn serialize_error_message_is_verbatim() {
        let err = StoreError::Serialize {
            detail: "serializer error".to_owned(),
        };

        assert_eq!(
            user_message(err),
            "序列化配置文件失败\n原因：serializer error"
        );
    }

    #[test]
    fn slug_and_source_error_messages_are_verbatim() {
        assert_eq!(
            user_message(StoreError::DuplicateSlug {
                slug: "foo".to_owned()
            }),
            "已存在同名 Provider：foo"
        );
        assert_eq!(
            user_message(StoreError::MissingSlug {
                slug: "ghost".to_owned()
            }),
            "Provider 不存在：ghost"
        );
        assert_eq!(
            user_message(StoreError::MissingSource {
                source: "builtin:pi".to_owned()
            }),
            "插件条目不存在：builtin:pi"
        );
        assert_eq!(
            user_message(StoreError::DuplicateSource {
                source: "https://example.com/manifest.json".to_owned()
            }),
            "已存在同一来源的插件条目：https://example.com/manifest.json"
        );
        assert_eq!(
            user_message(StoreError::DuplicateId {
                id: "pi".to_owned(),
                source: "builtin:pi".to_owned()
            }),
            "插件 id「pi」已被来源 builtin:pi 占用"
        );
    }

    #[test]
    fn invalid_variable_name_error_message_is_verbatim() {
        let err = StoreError::InvalidVariableName {
            name: "a-b".to_owned(),
        };

        assert_eq!(
            user_message(err),
            "变量名不合法：「a-b」\n变量名只能包含字母、数字与下划线（见 ADR 0014）。"
        );
    }

    /// 锁毒化与 `String` 错误域同句：两条加锁路径对用户是同一件事。
    #[test]
    fn lock_poisoned_folds_into_the_same_reason_for_both_error_domains() {
        assert_eq!(String::from(crate::store::LockPoisoned), "配置存储不可用");
        assert_eq!(
            user_message(StoreError::Unavailable),
            "配置存储不可用"
        );
    }
}
