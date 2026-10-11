//! 内置 Claude Code 插件：把 Maestro 录入的 Provider 投影为
//! `~/.claude/settings.json` 的 `env` 块。
//!
//! 投影合同（写哪些键、怎么选 Provider、失败语义、合并写入）见本目录的
//! README.md，决策见 ADR 0020、ADR 0021。宿主把 manifest 声明的 config_dir
//! （Claude Code 即 `$HOME/.claude`）预开放为 "/"，插件只写其中的
//! settings.json；该文件由 Claude Code 自身一并写入，故只增删本插件维护的键。

use maestro_plugin_sdk::{Guest, Provider, export};

struct ClaudeCodePlugin;

impl Guest for ClaudeCodePlugin {
    fn write_providers(_providers: Vec<Provider>) -> Result<Vec<String>, String> {
        todo!("投影逻辑见 plugins/claude-code/README.md，尚未实现")
    }
}

export!(ClaudeCodePlugin);
