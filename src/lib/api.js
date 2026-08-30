// Thin wrappers over the Tauri commands exposed by the Rust backend.
// The backend returns the full updated config after each mutation.

import { invoke } from "@tauri-apps/api/core";

export function getConfig() {
  return invoke("get_config");
}

export function saveProvider(provider) {
  return invoke("save_provider", { provider });
}

export function deleteProvider(id) {
  return invoke("delete_provider", { id });
}

export function savePlugin(plugin) {
  return invoke("save_plugin", { plugin });
}

export function deletePlugin(id) {
  return invoke("delete_plugin", { id });
}
