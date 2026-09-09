# 0005: 内置插件 WASM 产物不入库，宿主构建时自动编译

内置插件（见 ADR 0004）最初以编译产物 `plugins/pi/plugin.wasm` 提交入库，并由 CI 重新编译后做字节级比对防漂移——这要求构建字节级可复现（toolchain 锁定、`--remap-path-prefix` 抹平 sysroot 与 registry 路径）。实践中可复现构建本身很脆（registry src 目录 hash、rustc 重编译差异、CI 缓存状态等任一变动即比对失败），且开发者每次改插件源码都要手动重建并提交二进制：防漂移的机制反而成了漂移的来源。现决定：产物不再入库，由 `src-tauri/build.rs` 在构建宿主时自动编译插件（`cargo build --release --locked --target wasm32-wasip2`，产物写 `OUT_DIR`，`include_bytes!` 引用），`rerun-if-changed` 盯住插件目录；CI 不再比对产物字节，正确性由宿主测试（实例化真实编译产物并执行投影）保障。代价是构建宿主者承担插件编译（wasm32-wasip2 已列入 rust-toolchain.toml 的 targets，装工具链即自带）。Git 来源的第三方插件仓库不受此决定约束——其产物形态由插件作者自行决定，宿主仅在装载与安装时做实例化校验。

曾考虑并否决的形态：CI 机器人自动重建并提交产物（保留二进制入库与可复现构建要求，只把手动编译变成自动提交，脆弱的比对环仍在）；check 弱化为"重建后跑行为测试不比字节"（check 不再脆，但发布的是提交的旧字节、测试的是重建的新字节，引入发布物与测试物不一致）；继续字节级方案逐个修死漂移源（两个原始痛点原样保留）。
