import { useEffect, useState } from "react";
import { Layout, Menu } from "antd";
import PluginsPage from "@/pages/plugins";
import PluginConfigPage from "@/pages/plugin-config";
import ProvidersPage from "@/pages/provider";
import { listPlugins } from "@/api/plugins";
import { currentPluginOf, pluginNavItems } from "./navigation";
import "./App.css";

const { Sider, Content } = Layout;

// 侧边栏结构：品牌置顶，普通菜单居中，功能性入口置底（见 issue #34）。
const NAV_MAIN_ITEMS = [{ key: "providers", label: "Provider" }];
const NAV_BOTTOM_ITEMS = [{ key: "plugins", label: "Plugins" }];

function App() {
  const [current, setCurrent] = useState("providers");
  const [plugins, setPlugins] = useState([]);

  // 导航项随切页刷新：插件在 Plugins 页增删启停之后，重新进入任何页面时
  // 都取最新列表（list_plugins 是本地命令，代价可忽略）；取不到时保留旧列表，
  // 避免正在展示的插件配置页被清空。
  useEffect(() => {
    listPlugins()
      .then(setPlugins)
      .catch((err) => {
        console.warn("failed to refresh plugins for navigation", err);
      });
  }, [current]);

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
      <Sider theme="light" width={200} className="app-sider">
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
          {current === "providers" && <ProvidersPage />}
          {current === "plugins" && <PluginsPage />}
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
