//! 凭证适配器：`keychain` 桥接（WIT `maestro:plugin` v1.1.0）的宿主侧。
//!
//! 插件跑在 wasm32-wasip2 沙箱里、够不到系统凭证库，因此「把 API Key 写进目标工具
//! 自己的钥匙串条目」只能由宿主代劳：插件把 `(key, secret)` 交给宿主，宿主按插件
//! manifest 声明的 `keychain_namespace` 选中适配器，由适配器补齐该工具条目的其余
//! 字段（服务名、用户名、标签）并落成条目。桥接面故意只有 `(key, secret)`——条目的
//! 格式是目标工具的内部约定，插件既不该也无从指定。
//!
//! `keychain_namespace` 既是权限闸门（未声明即拒绝调用），也是跨工具事故的隔离带：
//! 适配器只会把条目落成自己那种工具的格式，插件写不出别的工具认识的条目。它不进入
//! key 本身——目标工具按自己的索引键（如 Zed 按 API 端点 URL）精确匹配条目，给 key
//! 加前缀反而会让工具读不到。
//!
//! 决策见 ADR 0013。

/// 一种目标工具的凭证适配器：持有该工具钥匙串条目的格式知识（内部常量、索引键
/// 映射、平台差异），把宿主收到的 `(key, secret)` 落成该工具自己的条目。
///
/// 适配器只在宿主侧实现：格式知识收在一处，目标工具改版失效时只需发宿主版。
/// 具体工具的适配器随对应插件另立 issue 落地（见 ADR 0013）。
pub trait KeychainAdapter: Sync {
    /// 写入一条凭证；同 key 重复写入按适配器自身语义覆盖，不做冲突报错。
    fn write(&self, key: &str, secret: &str) -> Result<(), String>;

    /// 删除一条凭证；key 不存在视为成功（幂等）。
    fn delete(&self, key: &str) -> Result<(), String>;
}

/// 宿主内置的适配器表：manifest 的 `keychain_namespace` → 适配器实现。
///
/// 表为空即任何 namespace 都「不认识」——拒绝是默认姿态，适配器随对应插件逐个登记。
static ADAPTERS: &[(&str, &dyn KeychainAdapter)] = &[];

/// `keychain.write`：按插件声明的 namespace 分发。
pub fn write(namespace: Option<&str>, key: &str, secret: &str) -> Result<(), String> {
    let (namespace, adapter) = dispatch(ADAPTERS, namespace)?;
    log::debug!("keychain write: namespace={namespace}, key={key}");
    adapter.write(key, secret)
}

/// `keychain.delete`：按插件声明的 namespace 分发。
pub fn delete(namespace: Option<&str>, key: &str) -> Result<(), String> {
    let (namespace, adapter) = dispatch(ADAPTERS, namespace)?;
    log::debug!("keychain delete: namespace={namespace}, key={key}");
    adapter.delete(key)
}

/// 按 namespace 选适配器，返回选中的 namespace 与其适配器。
///
/// 未声明与不认识的 namespace 都在这里拒绝；错误经 import 回到插件，由插件按条目
/// 容错（一条写失败不影响其余条目与配置文件投影，见 issue #81）。
///
/// 适配器表由参数传入而非直接读 [`ADAPTERS`]：分发与表内容各自可测。
fn dispatch<'a>(
    table: &'a [(&'a str, &'a dyn KeychainAdapter)],
    namespace: Option<&'a str>,
) -> Result<(&'a str, &'a dyn KeychainAdapter), String> {
    // 未声明 namespace 即未获授权：声明本身是权限闸门（见 ADR 0013）。
    let namespace =
        namespace.ok_or_else(|| "插件未声明 keychain_namespace，凭证桥接调用被拒".to_owned())?;
    table
        .iter()
        .find(|(name, _)| *name == namespace)
        .map(|(name, adapter)| (*name, *adapter))
        .ok_or_else(|| format!("宿主不认识的 keychain_namespace「{namespace}」"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 记录收到的调用：验证分发确实把调用交给了所声明 namespace 的那个适配器。
    #[derive(Default)]
    struct RecordingAdapter {
        calls: Mutex<Vec<String>>,
    }

    impl RecordingAdapter {
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl KeychainAdapter for RecordingAdapter {
        fn write(&self, key: &str, secret: &str) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("write {key} {secret}"));
            Ok(())
        }

        fn delete(&self, key: &str) -> Result<(), String> {
            self.calls.lock().unwrap().push(format!("delete {key}"));
            Ok(())
        }
    }

    #[test]
    fn keychain_call_is_rejected_when_no_namespace_is_declared() {
        let adapter = RecordingAdapter::default();
        let table = [("zed", &adapter as &dyn KeychainAdapter)];

        let Err(err) = dispatch(&table, None) else {
            panic!("未声明 namespace 的调用不应被分发");
        };
        assert!(err.contains("未声明 keychain_namespace"), "{err}");
        // 被拒的调用不得落到任何适配器上。
        assert!(adapter.calls().is_empty());
    }

    #[test]
    fn keychain_call_is_rejected_for_an_unknown_namespace() {
        let adapter = RecordingAdapter::default();
        let table = [("zed", &adapter as &dyn KeychainAdapter)];

        // 宿主内置表当前为空：任何 namespace 都不认识。
        let Err(err) = dispatch(ADAPTERS, Some("zed")) else {
            panic!("内置表为空时不应分发任何 namespace");
        };
        assert!(err.contains("不认识的 keychain_namespace「zed」"), "{err}");

        let Err(err) = dispatch(&table, Some("opencode")) else {
            panic!("表外的 namespace 不应被分发");
        };
        assert!(
            err.contains("不认识的 keychain_namespace「opencode」"),
            "{err}"
        );
        assert!(adapter.calls().is_empty());
    }

    #[test]
    fn keychain_call_is_dispatched_to_the_adapter_of_the_declared_namespace() {
        let zed = RecordingAdapter::default();
        let other = RecordingAdapter::default();
        let table = [
            ("zed", &zed as &dyn KeychainAdapter),
            ("other", &other as &dyn KeychainAdapter),
        ];

        let (namespace, adapter) = dispatch(&table, Some("other")).unwrap();
        assert_eq!(namespace, "other");

        adapter.write("api_url", "sk-projected").unwrap();
        adapter.delete("api_url").unwrap();

        assert_eq!(
            other.calls(),
            ["write api_url sk-projected", "delete api_url"]
        );
        assert!(zed.calls().is_empty(), "不得落到未声明的适配器上");
    }
}
