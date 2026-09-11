import { useCallback, useEffect, useState } from "react";
import {
  Alert,
  Button,
  Card,
  Empty,
  Flex,
  Form,
  Input,
  message,
  Modal,
  Popconfirm,
  Spin,
  Switch,
  Tag,
  Typography,
} from "antd";
import {
  addPlugin,
  listPlugins,
  reloadPlugins,
  removePlugin,
  setPluginEnabled,
  updatePlugin,
} from "./api/plugins";

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
      {!plugin.builtin && (
        <div className="card-actions">
          <Button
            disabled={busy}
            onClick={() =>
              run(() => updatePlugin(plugin.source), "已更新插件")
            }
          >
            更新
          </Button>
          <Popconfirm
            title="移除插件"
            description="将删除该插件的配置条目与已落位的插件文件，确定移除？"
            okText="移除"
            cancelText="取消"
            okButtonProps={{ danger: true }}
            onConfirm={() =>
              run(() => removePlugin(plugin.source), "已移除插件")
            }
          >
            <Button danger disabled={busy}>
              移除
            </Button>
          </Popconfirm>
        </div>
      )}
    </Card>
  );
}

const URL_RULES = [
  { required: true, message: "请输入 manifest.json 的 https 地址" },
  {
    validator: (_, value) => {
      if (!value) return Promise.resolve();
      let url;
      try {
        url = new URL(value);
      } catch {
        return Promise.reject(new Error("不是合法的 URL"));
      }
      if (url.protocol !== "https:") {
        return Promise.reject(new Error("插件来源仅支持 https 地址"));
      }
      return Promise.resolve();
    },
  },
];

/**
 * 添加插件对话框：本期只提供 https 来源（来源切换由 file 来源票引入）。
 *
 * @param {Object} param0
 * @param {boolean} param0.open
 * @param {() => void} param0.onClose
 * @param {(url: string) => Promise<void>} param0.onSubmit
 */
function AddPluginModal({ open, onClose, onSubmit }) {
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);

  async function handleSubmit() {
    let values;
    try {
      values = await form.validateFields();
    } catch {
      // 校验失败已内联展示。
      return;
    }
    setSaving(true);
    try {
      await onSubmit(values.url);
      onClose();
    } catch (err) {
      message.error(String(err));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Modal
      title="添加插件"
      open={open}
      okText="添加"
      cancelText="取消"
      confirmLoading={saving}
      destroyOnHidden
      onOk={handleSubmit}
      onCancel={onClose}
    >
      <Form form={form} layout="vertical" onFinish={handleSubmit}>
        <Form.Item
          name="url"
          label="https 地址"
          extra="指向插件 manifest.json 的 https 地址，例如 GitHub Release 上的 manifest.json"
          rules={URL_RULES}
        >
          <Input placeholder="https://github.com/owner/repo/releases/download/v1/manifest.json" />
        </Form.Item>
      </Form>
    </Modal>
  );
}

export default function PluginsPage() {
  const [plugins, setPlugins] = useState([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(null);
  const [adding, setAdding] = useState(false);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const list = await listPlugins();
      setPlugins(list);
      setLoadError(null);
      return list;
    } catch (err) {
      setLoadError(String(err));
      return [];
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    reload();
  }, [reload]);

  /** 条目先落盘再安装：安装失败也保留条目（错误态），因此这里总能刷新出结果。 */
  async function handleAdd(url) {
    await addPlugin(url);
    const list = await reload();
    const added = list.find((plugin) => plugin.source === url);
    if (added?.status === "error") {
      message.warning("插件条目已添加，但安装未完成：见卡片上的原因，可点「更新」重试");
    } else {
      message.success("已添加插件");
    }
  }

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
