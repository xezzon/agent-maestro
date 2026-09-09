import { useCallback, useEffect, useState } from "react";
import {
  Alert,
  Button,
  Card,
  Empty,
  Flex,
  message,
  Spin,
  Switch,
  Tag,
  Typography,
} from "antd";
import { listPlugins, reloadPlugins, setPluginEnabled } from "./api/plugins";

/**
 * @param {Object} param0
 * @param {import("./api/plugins").PluginView} param0.plugin
 * @param {() => void} param0.onReload
 */
function PluginCard({ plugin, onReload }) {
  const [busy, setBusy] = useState(false);

  async function run(action, successText) {
    setBusy(true);
    try {
      await action();
      if (successText) {
        message.success(successText);
      }
      onReload();
    } catch (err) {
      message.error(String(err));
    } finally {
      setBusy(false);
    }
  }

  const title = (
    <Flex justify="space-between" align="center">
      <Typography.Text strong>{plugin.name || plugin.source}</Typography.Text>
      <Switch
        checked={plugin.enabled}
        checkedChildren="启用"
        unCheckedChildren="禁用"
        disabled={busy}
        onChange={(enabled) =>
          run(() => setPluginEnabled(plugin.source, enabled))
        }
      />
    </Flex>
  );

  return (
    <Card title={title}>
      <div className="plugin-meta">
        <span>
          来源{" "}
          <Typography.Text copyable={{ text: plugin.source }}>
            {plugin.source}
          </Typography.Text>
        </span>
        {plugin.tool && <span>适配工具：{plugin.tool}</span>}
        {plugin.config_dir && <span>写入目录：{plugin.config_dir}</span>}
      </div>
      {plugin.status === "error" ? (
        <Alert
          className="plugin-error"
          type="error"
          showIcon
          title="加载失败"
          description={plugin.error}
        />
      ) : (
        <div className="plugin-status">
          <Tag color="success">已加载</Tag>
        </div>
      )}
    </Card>
  );
}

export default function PluginsPage() {
  const [plugins, setPlugins] = useState([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(null);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      setPlugins(await listPlugins());
      setLoadError(null);
    } catch (err) {
      setLoadError(String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    reload();
  }, [reload]);

  async function handleReloadRegistry() {
    setLoading(true);
    try {
      await reloadPlugins();
      message.success("已从磁盘重新加载插件");
      await reload();
    } catch (err) {
      message.error(String(err));
    } finally {
      setLoading(false);
    }
  }

  if (loadError) {
    return (
      <div className="page">
        <div className="page-header">
          <Typography.Title level={4} style={{ margin: 0 }}>
            Plugins
          </Typography.Title>
        </div>
        <Alert
          type="error"
          showIcon
          title="插件列表加载失败"
          description={<pre className="error-detail">{loadError}</pre>}
        />
      </div>
    );
  }

  return (
    <div className="page">
      <div className="page-header">
        <Typography.Title level={4} style={{ margin: 0 }}>
          Plugins
        </Typography.Title>
        <Flex gap={8}>
          <Button disabled={loading} onClick={handleReloadRegistry}>
            重新加载
          </Button>
        </Flex>
      </div>

      {loading && plugins.length === 0 ? (
        <div className="page-loading">
          <Spin />
        </div>
      ) : plugins.length === 0 ? (
        <Empty description="尚未安装任何插件" />
      ) : (
        <Flex vertical gap={16}>
          {plugins.map((plugin) => (
            <PluginCard
              key={plugin.source}
              plugin={plugin}
              onReload={reload}
            />
          ))}
        </Flex>
      )}
    </div>
  );
}
