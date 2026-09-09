#!/usr/bin/env bash
# 重建 pi 插件 WASM 组件并覆盖 plugins/pi/plugin.wasm。
# 仅插件作者与 CI 需要：要求已安装 wasm32-wasip2 target（rustup target add wasm32-wasip2），日常开发无需执行。
#
# 构建必须字节级可复现（CI 会比对提交的 plugin.wasm），因此：
# - 工具链版本由仓库根 rust-toolchain.toml 锁定；
# - sysroot 与 cargo registry 源码目录在本机间路径不同（长度也不同），
#   会以 panic 位置字符串的形式嵌入产物，这里统一重映射为机器无关的固定路径。
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$script_dir"

# 先下载依赖，保证 registry 源码目录存在（fresh 环境/缓存未命中时亦然）。
cargo fetch --locked

sysroot="$(rustc --print sysroot)"
registry_src="$(find "${CARGO_HOME:-$HOME/.cargo}/registry/src" -mindepth 1 -maxdepth 1 -type d | head -n1)"
export RUSTFLAGS="--remap-path-prefix=$sysroot=/sysroot --remap-path-prefix=$registry_src=/deps"

cargo build --release --locked --target wasm32-wasip2
cp -f target/wasm32-wasip2/release/maestro_plugin_pi.wasm plugin.wasm
