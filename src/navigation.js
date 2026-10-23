import { isUsablePlugin } from "@/api/plugins";

/**
 * 侧边栏的导航派生：从插件列表推出菜单项与当前插件。
 *
 * 这些派生此前内联在 `App.jsx` 里，零测试——「导航里有、插件页里没有」这类
 * 漂移只能靠人眼看出。派生是纯函数，单独成模块即可直接测。
 */

/** 插件配置页的导航 key 前缀，key 为 `plugin:<id>`（id 是插件唯一标识）。 */
export const PLUGIN_KEY_PREFIX = "plugin:";

/**
 * 插件的导航项：每个已启用且加载成功的插件各占一个菜单项，直达其插件配置页；
 * 已禁用或加载失败的插件不出现（ticket #91）。
 * @param {import("@/api/plugins").PluginView[]} plugins
 * @returns {{ key: string, label: string }[]}
 */
export function pluginNavItems(plugins) {
  return plugins.filter(isUsablePlugin).map((plugin) => ({
    key: `${PLUGIN_KEY_PREFIX}${plugin.id}`,
    label: plugin.id,
  }));
}

/**
 * 当前导航 key 指向的插件；不是插件配置页（或是已不在列表里的插件）时返回
 * `null`——界面据此不渲染插件配置页。
 * @param {import("@/api/plugins").PluginView[]} plugins
 * @param {string} current 当前导航 key
 * @returns {import("@/api/plugins").PluginView|null}
 */
export function currentPluginOf(plugins, current) {
  if (!current.startsWith(PLUGIN_KEY_PREFIX)) return null;
  // 前缀之后是插件 id（`PluginView.id`），不是 `source`（`builtin:<id>`）。
  const id = current.slice(PLUGIN_KEY_PREFIX.length);
  return plugins.find((plugin) => plugin.id === id) ?? null;
}
