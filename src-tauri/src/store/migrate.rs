//! 启动时迁移旧明文 API Key（#147）：尽力而为，失败保留，下次重试。
//!
//! 迁移在配置存储构造时立即执行（早于命令与插件启动）：用户升级后无需
//! 重新录入，配置里的旧明文一次性换进系统密钥链。
//!
//! 三条规则：
//!
//! - **逐 Provider 尽力而为**：成功的换成规范引用，失败的**保留明文原样**
//!   并记下原因——丢掉密钥比留在配置里更坏（ADR 0002）。
//! - **只落盘一次**：全部尝试完再写配置（成功的写引用、失败的写回明文），
//!   不一个 Provider 写一次。
//! - **失败不阻塞启动**：密钥链不可用（如 headless Linux 没有 Secret
//!   Service）时应用照常启动，只是配置里仍是明文，下次启动再试。
//!
//! 落盘失败时内存状态保持迁移前的样子（配置压根没被换掉），而本次写进
//! 密钥链的条目**不回收**：它们是惰性残余，同 slug 下次写入时即被覆盖
//! （ADR 0002 接受的风险）。

use crate::{
    config::Config,
    secret::{SecretStore, SecretText},
};

use super::secrets::canonical;

/// 一次迁移失败：哪个 Provider、为什么。
///
/// 原因只描述平台侧的失败，**不含密钥值**（ADR 0008）。
pub(super) struct Failure {
    /// Provider 的 slug。
    pub(super) slug: String,
    /// 失败原因。
    pub(super) reason: String,
}

/// 一次迁移的结果：迁成的 slug + 失败清单。
pub(super) struct Migration {
    /// 迁成了引用的 Provider 的 slug（空即「一个都没迁成」，不必落盘）。
    pub(super) migrated_slugs: Vec<String>,
    /// 每个失败者一条。
    pub(super) failures: Vec<Failure>,
}

/// 把配置里的旧明文 API Key 搬进密钥链。
///
/// 配置就地改写：换成功的字段变成规范引用，换不成的原样留着。禁用项也
/// 照迁——密钥该待在密钥链里，与它是否参与投影无关。
///
/// **不走** [`super::secrets::overwrite`]：它为「落盘失败」准备回滚（把条目
/// 恢复成旧值），而迁移的落盘失败**刻意**保留本次写入的条目（惰性残余，
/// 同 slug 下次写入时覆盖，ADR 0002 接受的风险）。这里的写入没有旧值要
/// 备份——旧值就在配置里的明文本身。
pub(super) fn migrate(config: &mut Config, secrets: &dyn SecretStore) -> Migration {
    let mut migrated_slugs = Vec::new();
    let mut failures = Vec::new();
    for (slug, provider) in config.providers.iter_mut() {
        // 明文先取出来（克隆）：下面要把整个字段换成引用，借用跨不过那次赋值。
        let Some(plaintext) = provider.api_key.plaintext().map(str::to_owned) else {
            continue;
        };
        // 引用与 slug 绑定：slug 不合法就构造不出引用，明文因此只能留在
        // 配置里（宁可报错，也不把真值退回配置文件，ADR 0002）。
        let reference = match canonical(slug) {
            Ok(reference) => reference,
            Err(error) => {
                failures.push(Failure {
                    slug: slug.clone(),
                    reason: error.to_string(),
                });
                continue;
            }
        };
        match secrets.set(&reference, &plaintext) {
            Ok(()) => {
                provider.api_key = SecretText::referenced(reference);
                migrated_slugs.push(slug.clone());
            }
            // 写不进去就保留明文：密钥就此还在，下次启动再试。
            Err(error) => failures.push(Failure {
                slug: slug.clone(),
                reason: error.cause().to_string(),
            }),
        }
    }
    Migration {
        migrated_slugs,
        failures,
    }
}
