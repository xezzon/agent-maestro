import { useState } from "react";
import {
  Alert,
  App,
  Button,
  Empty,
  Flex,
  Spin,
  Typography,
} from "antd";
import { addPlugin } from "@/api/plugins";
import AddPluginModal from "./AddPluginModal";
import PluginCard from "./PluginCard";

/**
 * 插件管理页：已安装插件的卡片 + 添加入口。
 *
 * 插件列表由 App 传入（全应用一份数据）：导航项、Provider 页的投影按钮与本页
 * 的卡片读的是同一份列表，不再各取一次快照。
 * @param {Object} param0
 * @param {import("@/api/plugins").PluginView[]} param0.plugins 插件列表
 * @param {boolean} param0.loading 列表加载中
 * @param {string=} param0.loadError 列表加载失败的原因
 * @param {() => Promise<import("@/api/plugins").PluginView[] | null>} param0.onReload 刷新列表
 */
export default function PluginsPage({
  plugins,
  loading,
  loadError,
  onReload,
}) {
  const [adding, setAdding] = useState(false);
  const { message } = App.useApp();

  /** 安装成功才写入条目：失败由命令层抛出原因（对话框内展示），这里只刷新成功的结果。 */
  async function handleAdd(source) {
    await addPlugin(source);
    const list = await onReload();
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
            <Button loading={loading} onClick={onReload}>
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
              onReload={onReload}
            />
          ))}
        </Flex>
      )}
    </div>
  );
}
