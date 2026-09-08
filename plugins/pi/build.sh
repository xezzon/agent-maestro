#!/usr/bin/env bash
# 重建 pi 插件 WASM 组件并覆盖 plugins/pi/plugin.wasm。
# 仅插件作者与 CI 需要：要求已安装 wasm32-wasip2 target（rustup target add wasm32-wasip2），日常开发无需执行。
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$script_dir"

cargo build --release --target wasm32-wasip2
cp -f target/wasm32-wasip2/release/maestro_plugin_pi.wasm plugin.wasm
