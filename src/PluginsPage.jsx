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

const GIT_SOURCE_RULES = [
  { required: true, message: "请输入 Git 仓库地址" },
  {
    validator: (_, value) =>
      value?.startsWith("https://") && value.length > "https://".length
        ? Promise.resolve()
        : Promise.reject(new Error("插件来源仅支持匿名 HTTPS 地址")),
  },
];

/**
 * 添加 Git 来源插件：先落配置条目，下载失败条目保留可重试。
 * @param {Object} param0
 * @param {(refresh: boolean) => void} param0.onFinish
 */
function AddPluginForm({ onFinish }) {
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);

  async function handleSave() {
    setSaving(true);
    try {
      const { source } = await form.validateFields();
      await addPlugin(source);
      message.success("插件已添加");
      onFinish(true);
    } catch (err) {
      // 校验失败已内联展示，无需重复报错；其余为命令调用失败。
      if (!err?.errorFields) {
        message.error(String(err));
      }
    } finally {
      setSaving(false);
    }
  }

  return (
    <Form form={form} layout="vertical">
      <Form.Item
        name="source"
        label="Git 仓库地址"
        extra="匿名 HTTPS、默认分支，仓库根目录即插件根目录"
        rules={GIT_SOURCE_RULES}
      >
        <Input placeholder="例如 https://github.com/user/maestro-plugin.git" />
      </Form.Item>
      <div className="card-actions">
        <Button disabled={saving} onClick={() => onFinish(false)}>
          取消
        </Button>
        <Button type="primary" loading={saving} onClick={handleSave}>
          添加
        </Button>
      </div>
    </Form>
  );
}

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
            onClick={() => run(() => updatePlugin(plugin.source), "已更新")}
          >
            更新
          </Button>
          <Popconfirm
            title="移除插件"
            description="将同时删除配置条目与插件目录，确定移除？"
            okText="移除"
            cancelText="取消"
            okButtonProps={{ danger: true }}
            onConfirm={() =>
              run(() => removePlugin(plugin.source), "已移除")
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

export default function PluginsPage() {
  const [plugins, setPlugins] = useState([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(null);
  const [adding, setAdding] = useState(false);

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
          <Button
            type="primary"
            disabled={adding || loading}
            onClick={() => setAdding(true)}
          >
            添加插件
          </Button>
        </Flex>
      </div>

      {loading && plugins.length === 0 ? (
        <div className="page-loading">
          <Spin />
        </div>
      ) : plugins.length === 0 && !adding ? (
        <Empty description="尚未安装任何插件">
          <Button type="primary" onClick={() => setAdding(true)}>
            添加插件
          </Button>
        </Empty>
      ) : (
        <Flex vertical gap={16}>
          {adding && (
            <Card title="添加插件">
              <AddPluginForm
                onFinish={(refresh) => {
                  setAdding(false);
                  if (refresh) {
                    reload();
                  }
                }}
              />
            </Card>
          )}
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
