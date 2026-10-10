use super::HostState;
use super::bindings;

/// config import（ADR 0018）：返回实例化前就位的插件配置 `form` 段原始 JSON
/// 文本，插件自行解析。宿主只透传不审查（信任模型见 SDK README）。
impl bindings::maestro::plugin::config::Host for HostState {
    fn get_config(&mut self) -> Option<String> {
        self.form.clone()
    }
}

#[cfg(test)]
mod tests {
    use wasmtime_wasi::{ResourceTable, WasiCtxBuilder};

    use super::super::bindings::maestro::plugin::config::Host;
    use super::super::{HostState, build_store_limits};

    fn host_state(form: Option<String>) -> HostState {
        HostState {
            table: ResourceTable::new(),
            ctx: WasiCtxBuilder::new().build(),
            limits: build_store_limits(),
            plugin_id: "pi".to_owned(),
            form,
        }
    }

    #[test]
    fn get_config_returns_the_form_json_text() {
        let form = r#"{"model":"claude-opus"}"#.to_owned();
        let mut state = host_state(Some(form.clone()));

        assert_eq!(state.get_config(), Some(form));
    }

    #[test]
    fn get_config_is_none_without_a_form() {
        let mut state = host_state(None);

        assert_eq!(state.get_config(), None);
    }
}
