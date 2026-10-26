import { useCallback, useEffect, useState } from "react";
import { Layout, Menu } from "antd";
import PluginsPage from "@/pages/plugins";
import PluginConfigPage from "@/pages/plugin-config";
import PreferencesPage from "@/pages/preferences";
import ProvidersPage from "@/pages/provider";
import { listPlugins } from "@/api/plugins";
import { usePreferences } from "@/preferences/PreferencesProvider";
import { currentPluginOf, pluginNavItems } from "./navigation";
import { createLatestRequestGate } from "./latestRequest";
import "./App.css";

const { Sider, Content } = Layout;

// 侧边栏结构：品牌置顶，普通菜单居中，功能性入口置底（见 issue #34）。
const NAV_MAIN_ITEMS = [{ key: "providers", label: "Provider" }];
const NAV_BOTTOM_ITEMS = [
  { key: "plugins", label: "Plugins" },
  { key: "preferences", label: "偏好" },
];

function App() {
  const [current, setCurrent] = useState("providers");
  // 侧边栏外观跟着解析后的偏好走（不再是硬编码的亮色）。
  const { appearance } = usePreferences();
  // 插件列表是全应用的一份数据：导航项、Provider 页的投影按钮与 Plugins 页
  // 的卡片读的都是它。此前 App 与 Provider 页各调一次 listPlugins，各持一份
  // 快照——判定（isUsablePlugin）共享了，数据没有。
  const [plugins, setPlugins] = useState([]);
  const [pluginsLoading, setPluginsLoading] = useState(true);
  const [pluginsError, setPluginsError] = useState(null);
  // 刷新闸门：切页刷新与 Plugins 页的显式刷新会重叠，较慢的旧请求可能在较新
  // 的请求之后落地，把插件列表覆盖成旧快照。只有最新一次请求的结果才写回。
  const [requestGate] = useState(createLatestRequestGate);

  /** 刷新插件列表：成功返回列表；失败置 pluginsError 并返回 `null`（区别于空列表）。 */
  const reloadPlugins = useCallback(async () => {
    const requestId = requestGate.begin();
    setPluginsLoading(true);
    try {
      const list = await listPlugins();
      // 已被更新的请求取代：结果过期，丢弃（不覆盖较新的列表，也不提前清掉 loading）。
      if (!requestGate.isLatest(requestId)) return null;
      setPlugins(list);
      setPluginsError(null);
      return list;
    } catch (err) {
      console.warn("failed to refresh plugins", err);
      if (!requestGate.isLatest(requestId)) return null;
      setPluginsError(String(err));
      return null;
    } finally {
      if (requestGate.isLatest(requestId)) setPluginsLoading(false);
    }
  }, [requestGate]);

  // 导航项随切页刷新：插件在 Plugins 页增删启停之后，重新进入任何页面时
  // 都取最新列表（list_plugins 是本地命令，代价可忽略）；取不到时保留旧列表，
  // 避免正在展示的插件配置页被清空。
  useEffect(() => {
    reloadPlugins();
  }, [current, reloadPlugins]);

  // 分隔线之后每个已启用且加载成功的插件各占一个菜单项，直达其插件配置页；
  // 已禁用或加载失败的插件不出现（ticket #91）。
  const navItems = pluginNavItems(plugins);
  const mainItems =
    navItems.length > 0
      ? [...NAV_MAIN_ITEMS, { type: "divider" }, ...navItems]
      : NAV_MAIN_ITEMS;

  const currentPlugin = currentPluginOf(plugins, current);

  return (
    <Layout className="app-shell">
      <Sider theme={appearance} width={200} className="app-sider">
        <div className="brand">Maestro</div>
        <Menu
          className="nav-main"
          mode="inline"
          items={mainItems}
          selectedKeys={[current]}
          onClick={({ key }) => setCurrent(key)}
        />
        <Menu
          className="nav-bottom"
          mode="inline"
          items={NAV_BOTTOM_ITEMS}
          selectedKeys={[current]}
          onClick={({ key }) => setCurrent(key)}
        />
      </Sider>
      <Layout>
        <Content className="app-content">
          {current === "providers" && <ProvidersPage plugins={plugins} />}
          {current === "plugins" && (
            <PluginsPage
              plugins={plugins}
              loading={pluginsLoading}
              loadError={pluginsError}
              onReload={reloadPlugins}
            />
          )}
          {current === "preferences" && <PreferencesPage />}
          {currentPlugin && (
            <PluginConfigPage
              key={currentPlugin.source}
              plugin={currentPlugin}
            />
          )}
        </Content>
      </Layout>
    </Layout>
  );
}

export default App;
