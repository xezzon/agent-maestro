//! 投影前解析出的密钥：Provider slug → 明文 / 未设置。
//!
//! 插件收到的凭证是**明文**（WIT 合同不变），密钥引用绝不进插件
//! （ADR 0002）：解析发生在投影之前，插件映射只拿到「哪个 Provider 配了
//! 哪个密钥」，拿不到引用，也就没有把引用当密钥交出去的机会。
//!
//! 明文只在内存里从密钥链走到插件：Debug 只渲染「配没配」，绝不渲染明文
//! （ADR 0008：凭证绝不落盘进日志）。
//!
//! 解析的粒度是**每个 Provider 一次**：待投影的 Provider 逐个取回真值，任一
//! 取不到就整体失败（见 [`resolve`]）。

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::{self, Debug},
};

use crate::{
    errors::{PluginError, SecretCause},
    plugin::ports::SecretSource,
    provider::Provider,
};

/// 已解析的密钥表：Provider slug → 明文 / 未设置。
///
/// 「未设置」是一等状态：本地网关（如 Ollama）本就没有凭证，插件收到的是
/// `none`（WIT 合同如此）。
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ResolvedSecrets {
    entries: BTreeMap<String, Option<String>>,
}

impl ResolvedSecrets {
    /// 空的密钥表：没有 Provider 配了凭证。
    pub(crate) fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// 记下一个 Provider 的凭证：明文或未设置（`None`）。
    pub(crate) fn insert(&mut self, slug: String, secret: Option<String>) {
        self.entries.insert(slug, secret);
    }

    /// 某 Provider 的密钥明文：未设置或不在表里即 `None`。
    pub(crate) fn get(&self, slug: &str) -> Option<&str> {
        self.entries.get(slug).and_then(|value| value.as_deref())
    }
}

impl Debug for ResolvedSecrets {
    /// 只渲染「哪个 Provider 配了密钥」：明文绝不进 Debug 输出（ADR 0008）。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(
                self.entries
                    .iter()
                    .map(|(slug, secret)| (slug, secret.is_some())),
            )
            .finish()
    }
}

/// 投影前解析密钥：待投影的 Provider 逐个取回真值。
///
/// 谁参与解析：**启用且会被投影**的 Provider——`required` 是逐插件投影表里
/// 「端点非空」者的并集（见 `PluginService::required_slugs`）：任一会投影它的
/// 插件都算数，零端点的 Provider 本来就会被插件映射跳过，禁用的压根不参与
/// 投影：两者都不该因密钥问题阻塞「应用到工具」。
///
/// 失败即整体失败：任一 Provider 的密钥取不到（条目不存在）或密钥链不可用，
/// 整个解析失败并点名该 Provider——绝不把引用 URI 或空值交给工具
/// （ADR 0002）。两种病因分得开：「条目不存在」请用户重新设置，「密钥链
/// 不可用」请用户修后端。
pub(crate) fn resolve(
    providers: &BTreeMap<String, Provider>,
    required: &BTreeSet<String>,
    secrets: &impl SecretSource,
) -> Result<ResolvedSecrets, PluginError> {
    let mut resolved = ResolvedSecrets::new();
    for (slug, provider) in providers {
        // 禁用项由端口契约挡在外面（快照只含启用的 Provider），这里再问一遍：
        // 规则住在它执行的地方，端口契约被改坏时也不会把禁用项带进投影。
        if !provider.enabled || !required.contains(slug) {
            continue;
        }
        resolved.insert(slug.clone(), resolve_one(slug, provider, secrets)?);
    }
    Ok(resolved)
}

/// 单个 Provider 的凭证：引用去密钥链取真值，明文原样用，未设置即没有。
///
/// 三个取值口（[`SecretText::plaintext`](crate::secret::SecretText::plaintext) /
/// [`SecretText::reference`](crate::secret::SecretText::reference)）把三态分得
/// 开：这里逐个问一遍，不去匹配枚举变体——形态怎么变，问出来的结果都不变。
fn resolve_one(
    slug: &str,
    provider: &Provider,
    secrets: &impl SecretSource,
) -> Result<Option<String>, PluginError> {
    // 旧明文（迁移失败的配置）：原样用。启动迁移（#147）已把它换进密钥链，
    // 换不进去的才留到投影这一层。
    if let Some(value) = provider.api_key.plaintext() {
        return Ok(Some(value.to_owned()));
    }
    // 未设置：本地网关本就没有凭证，插件收到 none（WIT 合同如此）。
    let Some(reference) = provider.api_key.reference() else {
        return Ok(None);
    };
    // 引用：真值在密钥链里，取不回来即整体失败。
    match secrets.secret(reference) {
        Ok(Some(value)) => Ok(Some(value)),
        Ok(None) => Err(PluginError::secret_missing(slug)),
        Err(error) => Err(match error.cause() {
            // 「没有这一条」与读取失败是两种病因：前者请用户重新设置，后者请
            // 用户修密钥链（ADR 0002）。
            SecretCause::NoEntry => PluginError::secret_missing(slug),
            cause => PluginError::secret_unavailable(slug, cause.to_string()),
        }),
    }
}
