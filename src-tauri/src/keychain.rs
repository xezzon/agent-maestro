//! 凭证适配器：`keychain` 桥接（WIT `maestro:plugin` v1.1.0）的宿主侧。
//!
//! 插件跑在 wasm32-wasip2 沙箱里、够不到系统凭证库，因此「把 API Key 写进目标工具
//! 自己的钥匙串条目」只能由宿主代劳：插件把 `(key, secret)` 交给宿主，宿主按插件
//! manifest 声明的 `keychain_namespace` 把条目落进该 namespace（系统凭证库里的
//! service 名，目标工具自己的约定）。桥接面故意只有 `(key, secret)`——条目的格式是
//! 目标工具的内部约定，插件既不该也无从指定。
//!
//! `keychain_namespace` 是权限闸门：未声明即拒绝调用。声明值由插件作者给出、就是条目
//! 身份——宿主不预置 (工具 → 格式) 表，认错了名字（多加了前缀、认错了工具）目标工具
//! 就读不到条目，属静默失效。它不进入 key 本身：key 是目标工具侧的索引键（如 Zed 按
//! API 端点 URL 精确匹配条目），加前缀同样会让工具读不到。
//!
//! 条目最终落在系统凭证库（macOS Keychain / Windows 凭据管理器 / Linux Secret
//! Service）里，宿主侧的读写由 keyring-rs 承担（[`KeyringAdapter`]）。
//!
//! 决策见 ADR 0013。

/// 一种目标工具的凭证适配器：把宿主收到的 `(key, secret)` 落成目标工具在系统凭证库
/// 里的条目。
///
/// 适配器只在宿主侧实现：条目格式与 keyring 直接映射（见 [`KeyringAdapter`]）不同的
/// 工具（如整份凭证存成一条 JSON）要另立适配器——格式知识收在宿主一处，工具改版失效
/// 时只需发宿主版（见 ADR 0013）。
pub trait KeychainAdapter: Sync {
    /// 写入一条凭证；同 key 重复写入按适配器自身语义覆盖，不做冲突报错。
    fn write(&self, key: &str, secret: &str) -> Result<(), String>;

    /// 删除一条凭证；key 不存在视为成功（幂等）。
    fn delete(&self, key: &str) -> Result<(), String>;
}

/// 宿主侧以 keyring-rs 读写系统凭证库的实现。
mod keyring_store {
    use super::KeychainAdapter;

    /// 以系统凭证库（keyring-rs）承载条目的适配器。
    ///
    /// 条目由 `(service, account)` 定位、另带一份秘密：`service` 就是 manifest 声明的
    /// `keychain_namespace`（目标工具自己的约定），插件给的 `key` 是目标工具侧的索引键、
    /// 填 account 槽位，`secret` 填秘密槽位。索引键聚合成一份荷载（如一条 JSON 记录）的
    /// 工具用不上这个直接映射——那种条目得先读出荷载再合并写回，由该工具自己的适配器
    /// 实现。
    pub struct KeyringAdapter {
        /// 目标工具在系统凭证库里的 service 名（= manifest 声明的 namespace）。
        service: String,
    }

    impl KeyringAdapter {
        /// `service` 是目标工具在系统凭证库里的 service 名。
        pub fn new(service: impl Into<String>) -> Self {
            Self {
                service: service.into(),
            }
        }

        /// 取指向目标条目的凭证库句柄；顺带确保默认 store 已装好。
        fn entry(&self, key: &str) -> Result<keyring_core::Entry, String> {
            ensure_store()?;
            keyring_core::Entry::new(&self.service, key).map_err(|e| {
                format!(
                    "无法访问系统凭证库条目（service={}, key={key}）：{e}",
                    self.service
                )
            })
        }
    }

    impl KeychainAdapter for KeyringAdapter {
        fn write(&self, key: &str, secret: &str) -> Result<(), String> {
            log::debug!("keyring write: service={}, key={key}", self.service);
            self.entry(key)?.set_password(secret).map_err(|e| {
                format!(
                    "写入系统凭证库条目失败（service={}, key={key}）：{e}",
                    self.service
                )
            })
        }

        fn delete(&self, key: &str) -> Result<(), String> {
            log::debug!("keyring delete: service={}, key={key}", self.service);
            match self.entry(key)?.delete_credential() {
                // keychain.delete 的契约是幂等：条目本就不存在视为成功（见 ADR 0013）。
                Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
                Err(e) => Err(format!(
                    "删除系统凭证库条目失败（service={}, key={key}）：{e}",
                    self.service
                )),
            }
        }
    }

    /// 确保 keyring 有一个默认 store：已经装好（调用方预装、或测试的 mock）就不动，
    /// 否则由 keyring 门面按平台装上系统凭证库。装不上只尝试一次（结果被门面缓存），
    /// 返回的原因由各次调用带给插件。
    ///
    /// 条目走 keyring-core 而不是门面的 `Entry`：门面在平台凭证库装不上时一律拒绝建
    /// 条目，连调用方自备的 store 也不认——本机没有 Secret Service 时（CI、headless）
    /// 就用不了内存 mock，适配器也就无从测试。
    fn ensure_store() -> Result<(), String> {
        if keyring_core::get_default_store().is_some() {
            return Ok(());
        }
        match keyring::Entry::store_status() {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("系统凭证库不可用：{e}")),
        }
    }
}

/// `keychain.write`：把凭证落进插件声明的 namespace。
pub fn write(namespace: Option<&str>, key: &str, secret: &str) -> Result<(), String> {
    let namespace = declared(namespace)?;
    log::debug!("keychain write: namespace={namespace}, key={key}");
    keyring_store::KeyringAdapter::new(namespace).write(key, secret)
}

/// `keychain.delete`：删除插件声明的 namespace 下的凭证。
pub fn delete(namespace: Option<&str>, key: &str) -> Result<(), String> {
    let namespace = declared(namespace)?;
    log::debug!("keychain delete: namespace={namespace}, key={key}");
    keyring_store::KeyringAdapter::new(namespace).delete(key)
}

/// 权限闸门：未声明 namespace 即拒绝调用，拒绝是默认姿态（见 ADR 0013）。
///
/// 声明值即条目所在的 namespace，适配器在调用时按它取——宿主不预置 (工具 → 格式) 表，
/// 因此没有「宿主不认识」这一档。错误经 import 回到插件，由插件按条目容错（一条写失败
/// 不影响其余条目与配置文件投影，见 issue #81）。
fn declared(namespace: Option<&str>) -> Result<&str, String> {
    namespace.ok_or_else(|| "插件未声明 keychain_namespace，凭证桥接调用被拒".to_owned())
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::sync::{Mutex, MutexGuard};

    /// keyring 的默认 store 是进程级全局量：装 mock store 的测试要以同一把锁串起来，
    /// 否则并发测试互相清空条目。
    static STORE_LOCK: Mutex<()> = Mutex::new(());

    /// 装一份全新的内存 mock store（keyring-core 自带，见其 `mock` 模块），返回串行
    /// 守卫；mock 只在内存里，drop 即清空。真实凭证库在 CI/headless 上不可用，适配器的
    /// 测试一律走它。
    pub fn mock_store() -> MutexGuard<'static, ()> {
        let guard = STORE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        keyring_core::set_default_store(keyring_core::mock::Store::new().unwrap());
        guard
    }

    /// 直读 mock store 里的条目秘密：断言条目落没落在预期的 `(service, account)` 上。
    pub fn entry_secret(service: &str, key: &str) -> keyring_core::Result<String> {
        keyring_core::Entry::new(service, key)
            .unwrap()
            .get_password()
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::{entry_secret, mock_store};
    use super::*;

    #[test]
    fn keychain_call_is_rejected_when_no_namespace_is_declared() {
        let Err(err) = write(None, "api_url", "sk-projected") else {
            panic!("未声明 namespace 的调用不应被受理");
        };
        assert!(err.contains("未声明 keychain_namespace"), "{err}");
    }

    #[test]
    fn declared_namespace_is_where_the_entry_lands() {
        let _guard = mock_store();

        write(Some("zed"), "https://api.example.com/v1", "sk-secret").unwrap();

        assert_eq!(
            entry_secret("zed", "https://api.example.com/v1").unwrap(),
            "sk-secret"
        );
        // 声明值就是条目身份：换个 namespace 查不到这条。
        assert!(matches!(
            entry_secret("opencode", "https://api.example.com/v1"),
            Err(keyring_core::Error::NoEntry)
        ));
    }

    #[test]
    fn writing_the_same_key_again_overwrites_the_entry() {
        let _guard = mock_store();

        write(Some("zed"), "api_url", "sk-first").unwrap();
        write(Some("zed"), "api_url", "sk-second").unwrap();

        assert_eq!(entry_secret("zed", "api_url").unwrap(), "sk-second");
    }

    #[test]
    fn delete_removes_the_entry_and_is_idempotent() {
        let _guard = mock_store();

        // 条目本就不存在：删除视为成功，且不产生条目。
        delete(Some("zed"), "api_url").unwrap();
        assert!(matches!(
            entry_secret("zed", "api_url"),
            Err(keyring_core::Error::NoEntry)
        ));

        write(Some("zed"), "api_url", "sk-secret").unwrap();
        delete(Some("zed"), "api_url").unwrap();
        assert!(
            matches!(
                entry_secret("zed", "api_url"),
                Err(keyring_core::Error::NoEntry)
            ),
            "delete 必须真的删掉条目"
        );
        // 再删一次仍成功。
        delete(Some("zed"), "api_url").unwrap();
    }

    #[test]
    fn a_failing_store_is_reported_without_echoing_the_secret() {
        let _guard = mock_store();
        // mock store 支持给条目注入错误：下一次调用返回它。
        keyring_core::Entry::new("zed", "api_url")
            .unwrap()
            .as_any()
            .downcast_ref::<keyring_core::mock::Cred>()
            .unwrap()
            .set_error(keyring_core::Error::NoEntry);

        let err = write(Some("zed"), "api_url", "sk-secret").unwrap_err();

        assert!(err.contains("写入系统凭证库条目失败"), "{err}");
        assert!(!err.contains("sk-secret"), "错误消息不得回显 secret：{err}");
    }
}
