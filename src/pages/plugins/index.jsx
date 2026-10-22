import { useCallback, useEffect, useState } from "react";
import {
  Alert,
  Button,
  Empty,
  Flex,
  message,
  Spin,
  Typography,
} from "antd";
import { addPlugin, listPlugins } from "../../api/plugins";
import AddPluginModal from "./AddPluginModal";
import PluginCard from "./PluginCard";

export default function PluginsPage() {
  const [plugins, setPlugins] = useState([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(null);
  const [adding, setAdding] = useState(false);

  /** 刷新插件列表：成功返回列表；失败置 loadError 并返回 `null`（区别于空列表）。 */
  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const list = await listPlugins();
      setPlugins(list);
      setLoadError(null);
      return list;
    } catch (err) {
      setLoadError(String(err));
      // 刷新失败与「列表为空」必须可区分：调用方不得把失败当成安装成功。
      return null;
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    reload();
  }, [reload]);

  /** 安装成功才写入条目：失败由命令层抛出原因（对话框内展示），这里只刷新成功的结果。 */
  async function handleAdd(source) {
    await addPlugin(source);
    const list = await reload();
    // 刷新失败已展示错误；列表中不见新条目也不谎报成功。
    if (list === null) return;
    if (!list.some((plugin) => plugin.source === source)) return;
    message.success("已添加插件");
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
          action={
            // 加载失败不该把插件管理堵死到重新挂载页面：给一条重试路径。
            <Button loading={loading} onClick={reload}>
              重试
            </Button>
          }
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
          <Button
            type="primary"
            disabled={loading || adding}
            onClick={() => setAdding(true)}
          >
            添加插件
          </Button>
        </Flex>
      </div>

      <AddPluginModal
        open={adding}
        onClose={() => setAdding(false)}
        onSubmit={handleAdd}
      />

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
