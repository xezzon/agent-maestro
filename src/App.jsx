import { useState } from "react";
import { Layout, Menu } from "antd";
import PluginsPage from "./PluginsPage";
import ProvidersPage from "./ProvidersPage";
import "./App.css";

const { Sider, Content } = Layout;

// 侧边栏结构：品牌置顶，普通菜单居中，功能性入口置底（见 issue #34）。
const NAV_MAIN_ITEMS = [{ key: "providers", label: "Provider" }];
const NAV_BOTTOM_ITEMS = [{ key: "plugins", label: "Plugins" }];

function App() {
  const [current, setCurrent] = useState("providers");

  return (
    <Layout className="app-shell">
      <Sider theme="light" width={200} className="app-sider">
        <div className="brand">Maestro</div>
        <Menu
          className="nav-main"
          mode="inline"
          items={NAV_MAIN_ITEMS}
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
        </Content>
      </Layout>
    </Layout>
  );
}

export default App;
