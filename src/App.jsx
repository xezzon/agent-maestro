import { useState } from "react";
import { Layout, Menu } from "antd";
import ProvidersPage from "./ProvidersPage";
import "./App.css";

const { Sider, Content } = Layout;

const NAV_ITEMS = [{ key: "providers", label: "Provider" }];

function App() {
  const [current, setCurrent] = useState("providers");

  return (
    <Layout className="app-shell">
      <Sider theme="light" width={200} className="app-sider">
        <div className="brand">Maestro</div>
        <Menu
          mode="inline"
          items={NAV_ITEMS}
          selectedKeys={[current]}
          onClick={({ key }) => setCurrent(key)}
        />
      </Sider>
      <Layout>
        <Content className="app-content">
          {current === "providers" && <ProvidersPage />}
        </Content>
      </Layout>
    </Layout>
  );
}

export default App;
