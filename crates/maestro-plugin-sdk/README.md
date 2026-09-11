# maestro-plugin-sdk

Maestro 插件接口合同 `maestro:plugin` 的类型化 Rust 绑定。插件作者只依赖本 crate，实现 re-export 的 `Guest` trait 并以 `export!` 声明导出类型，即可构建 WASM 组件，无需 vendor WIT 文件。

## 依赖方式

本 crate 不发布到 crates.io。以 git 依赖引用本仓库中的 `crates/maestro-plugin-sdk`，并锁定 Maestro 的发布 tag——tag 与宿主版本一致，随 tag 固化插件与宿主的兼容组合：

```toml
[dependencies]
maestro-plugin-sdk = { git = "https://github.com/xezzon/agent-maestro", tag = "vX.Y.Z" }
```

tag 取含本 crate 的 release tag（`v0.1.0` 早于本 crate，需其后第一个 release）。

## 快速开始

```rust
use maestro_plugin_sdk::{export, Guest, Provider};

struct MyPlugin;

impl Guest for MyPlugin {
    fn write_providers(providers: Vec<Provider>) -> Result<Vec<String>, String> {
        // 把 Provider 投影进插件 manifest 声明的 config_dir（宿主预开放为 "/"），
        // 返回写入的文件路径。
        Ok(Vec::new())
    }
}

export!(MyPlugin);
```

`Provider` 每条只带一个协议的端点（见 ADR 0003）。写入应落在 manifest 声明的 `config_dir`——宿主把它预开放为 "/"，是插件唯一可写面；建议 tmp + rename 原子写。

## 构建

- 需要 Rust stable（`rust-version = 1.85.0`）与 target `wasm32-wasip2`。
- 产物是 WASM 组件（Component Model / WASI 0.2），由插件 manifest 的 `entry` 引用。

## 参考

- 内置 pi 插件（`plugins/pi`）是参考实现，兼作本 crate 的常驻契约回归。
- WIT 合同随本 crate 分发（`wit/maestro-plugin.wit`），构建时内嵌生成绑定（`build.rs`）；版本约定见 crate 文档（`src/lib.rs`）。
