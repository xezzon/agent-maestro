mod logger;
mod provider;
mod bindings {
    wasmtime::component::bindgen!({
        path: "../crates/maestro-plugin-sdk/wit",
        world: "plugin-world",
    });
}

pub(crate) use provider::SkippedProvider;
use std::path::Path;
use wasmtime::{
    Engine, StoreLimits, StoreLimitsBuilder,
    component::{Component, HasSelf, Linker},
};
use wasmtime_wasi::{FsPerms, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView, p2};

use super::LoadedPlugin;

/// 单次插件调用的执行预算（fuel）：投影任务的量级远小于该值，
/// 死循环或超量计算的插件会被中断并走失败路径，不会卡死投影。
const FUEL_BUDGET: u64 = 1 << 30;

/// 资源限制（防止恶意 memory.grow / table.grow 风暴，CWE-770）：
/// - 线性内存 64 MiB：覆盖一份 JSON 投影产物的体量上限。
/// - 表元素 1024：覆盖组件模型对 funcref 的一般需求。
/// - 单个组件典型会派生 2~4 个内部 core instance（host + component + 内嵌模块），
///   留出 8 以容纳更深的嵌套同时仍对实例数封顶。
/// - 单个组件：内存数 1（典型配置）；再小会与现有合法组件冲突。
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
const TABLE_ELEMENTS_LIMIT: usize = 1024;
const MEMORY_COUNT_LIMIT: usize = 1;
const INSTANCE_LIMIT: usize = 8;

/// 构建资源限制。
fn build_store_limits() -> StoreLimits {
    StoreLimitsBuilder::new()
        .memory_size(MEMORY_LIMIT)
        .table_elements(TABLE_ELEMENTS_LIMIT)
        .memories(MEMORY_COUNT_LIMIT)
        .instances(INSTANCE_LIMIT)
        .build()
}

/// WASI 宿主状态：唯一被授权的写入面是预开放的插件配置目录。
struct HostState {
    table: ResourceTable,
    ctx: WasiCtx,
    /// 资源限制：注册到 wasmtime Store，约束 wasm 线性内存、表、实例增长。
    limits: StoreLimits,
    /// 插件 id：应用日志行的归属标识（`log` import 无法从调用方推断）。
    plugin_id: String,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

impl HostState {
    /// 预开放指定配置目录为写入面，配套资源限制；用于实投影阶段。
    fn new(tool_dir: &Path, plugin_id: String) -> Result<Self, String> {
        let mut builder = WasiCtxBuilder::new();
        builder
            .preopened_dir(tool_dir, "/", FsPerms::ReadWrite)
            .map_err(|e| format!("无法开放插件配置目录：{e}"))?;
        Ok(Self {
            table: ResourceTable::new(),
            ctx: builder.build(),
            limits: build_store_limits(),
            plugin_id,
        })
    }
}

/// 宿主侧链接器：WASI 环境 + 本合同的 import。
///
/// `log` 恒定提供（见 ADR 0013）：组件不引用该 import 即不受影响；但 WIT 包
/// 版本升级后宿主只注册当前版本，按旧版合同编译的组件仍需重编译。
/// 校验期与实投影期共用同一份配置，避免两条路径的 import 集合漂移。
fn build_linker(engine: &Engine) -> Result<Linker<HostState>, String> {
    let mut linker: Linker<HostState> = Linker::new(engine);
    p2::add_to_linker_sync(&mut linker).map_err(|e| format!("初始化 WASI 宿主环境失败：{e}"))?;
    bindings::PluginWorld::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)
        .map_err(|e| format!("初始化插件日志 import 失败：{e}"))?;
    Ok(linker)
}

/// 已实例化的插件：实例化验证与实际调用共用同一管线。
pub(crate) struct InstantiatedPlugin {
    store: wasmtime::Store<HostState>,
    world: bindings::PluginWorld,
}

impl LoadedPlugin {
    /// 链接期校验：组件字节反序列化、导入/导出类型匹配，不触发组件 init、
    /// 不预开放真实配置目录。这是安装期使用的入口（见 ADR 0006：fail-fast）。
    pub(crate) fn validate(&self, engine: &Engine) -> Result<(), String> {
        let component =
            Component::new(engine, &self.wasm).map_err(|e| format!("不是有效的 WASM 组件：{e}"))?;
        let linker = build_linker(engine)?;
        // 组件模型的 instantiate_pre 不接收 Store：返回 Pre<()> 表示类型检查通过，
        // 不触发组件 init、不调用 host 函数。这正是安装期校验所需的最小集合。
        linker
            .instantiate_pre(&component)
            .map_err(|e| format!("插件接口不兼容：{e}"))?;
        log::debug!("plugin validated: id={}", self.manifest.id);
        Ok(())
    }

    /// 完整实例化：用于实投影（write_providers）。会预开放真实配置目录、
    /// 注册资源限制（防止恶意 memory.grow / table.grow 风暴，CWE-770）。
    pub(crate) fn instantiate_component(
        &self,
        engine: &Engine,
    ) -> Result<InstantiatedPlugin, String> {
        let component =
            Component::new(engine, &self.wasm).map_err(|e| format!("不是有效的 WASM 组件：{e}"))?;
        // 预开放的写入面是装载时解析出的宿主绝对路径（见 `resolve_config_dir`）。
        let tool_path = self.manifest.config_dir.clone();
        let mut store = wasmtime::Store::new(
            engine,
            HostState::new(&tool_path, self.manifest.id.clone())?,
        );
        store.limiter(|s| &mut s.limits);
        store
            .set_fuel(FUEL_BUDGET)
            .map_err(|e| format!("设置插件执行预算失败：{e}"))?;
        let linker = build_linker(engine)?;
        let world = bindings::PluginWorld::instantiate(&mut store, &component, &linker)
            .map_err(|e| format!("插件接口不兼容：{e}"))?;
        Ok(InstantiatedPlugin { store, world })
    }
}

#[cfg(test)]
mod testutil {
    use std::path::PathBuf;

    use super::*;
    use crate::plugin::{PlacedPlugin, PlatformDirs, testutil};

    /// 已装载的插件：走真实装载路径（`PlacedPlugin::load` 解析 `config_dir`）。
    /// manifest 声明 `$HOME/.pi`，平台基准目录全部指向传入的临时目录，解析结果
    /// 随插件一并返回——断言对着它写，路径不必硬编码两次。
    pub(crate) fn loaded_plugin(root: &Path, wasm: &[u8]) -> (LoadedPlugin, PathBuf) {
        let dirs = PlatformDirs::new(
            root.to_path_buf(),
            root.to_path_buf(),
            root.to_path_buf(),
            root.to_path_buf(),
            root.to_path_buf(),
        );
        PlatformDirs::init_test(dirs);
        let manifest = testutil::manifest_json("pi", "$HOME/.pi", "plugin.wasm");
        let loaded = PlacedPlugin::new(&root.join("placed"), manifest, wasm)
            .load()
            .unwrap();
        let config_dir = loaded.manifest.config_dir.clone();
        (loaded, config_dir)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::testutil::loaded_plugin;
    use crate::plugin::{build_engine, builtin};

    #[test]
    fn instantiate_rejects_bytes_that_are_not_a_component() {
        let root = tempfile::tempdir().unwrap();

        let (plugin, _) = loaded_plugin(root.path(), b"not a wasm component");
        let Err(err) = plugin.instantiate_component(&build_engine()) else {
            panic!("非组件字节不应通过实例化校验");
        };

        assert!(err.contains("不是有效的 WASM 组件"), "{err}");
    }

    #[test]
    fn validate_rejects_bytes_that_are_not_a_component() {
        let root = tempfile::tempdir().unwrap();

        let (plugin, _) = loaded_plugin(root.path(), b"not a wasm component");
        let Err(err) = plugin.validate(&build_engine()) else {
            panic!("非组件字节不应通过 validate");
        };

        assert!(err.contains("不是有效的 WASM 组件"), "{err}");
    }

    #[test]
    fn validate_accepts_a_valid_component() {
        let root = tempfile::tempdir().unwrap();

        let (plugin, _) = loaded_plugin(root.path(), builtin::PI_WASM);
        plugin
            .validate(&build_engine())
            .expect("内置 pi 应通过 validate");
    }

    /// 关键安全约束：链接期校验不触发组件 init、不预开放真实配置目录。
    /// 若实现回退到完整实例化，下面的标记文件会被 init 代码读写。
    #[test]
    fn validate_does_not_touch_the_real_config_dir() {
        let root = tempfile::tempdir().unwrap();
        // 标记文件落在装载解析出的 config_dir 内：正是投影时预开放为组件 `/` 的目录。
        let (plugin, config_dir) = loaded_plugin(root.path(), builtin::PI_WASM);
        let marker = config_dir.join("marker.txt");
        let original = "original-content";
        fs::write(&marker, original).unwrap();

        plugin.validate(&build_engine()).expect("validate 应通过");

        assert_eq!(
            fs::read_to_string(&marker).unwrap(),
            original,
            "validate 不得触碰真实配置目录"
        );
    }

    #[test]
    fn wasm_call_is_interrupted_when_fuel_exhausted() {
        let root = tempfile::tempdir().unwrap();
        let (plugin, _) = loaded_plugin(root.path(), builtin::PI_WASM);
        let mut plugin = plugin.instantiate_component(&build_engine()).unwrap();

        // 预算归零：第一条 guest 指令即触发 fuel 耗尽中断，调用转错误路径。
        plugin.store.set_fuel(0).unwrap();
        let handle = plugin.world.maestro_plugin_plugin();
        let err = handle
            .call_write_providers(&mut plugin.store, &[])
            .expect_err("fuel 耗尽应中断 wasm 调用");
        assert!(format!("{err:#}").contains("fuel"), "实际错误：{err:#}");
    }
}
