import { useState } from "react";
import {
  Alert,
  Button,
  Card,
  Flex,
  message,
  Popconfirm,
  Switch,
  Tag,
  Tooltip,
  Typography,
} from "antd";
import { FolderOpenOutlined } from "@ant-design/icons";
import { openPath } from "@tauri-apps/plugin-opener";
import {
  reloadPlugin,
  removePlugin,
  setPluginEnabled,
} from "../../api/plugins";

/**
 * @param {Object} param0
 * @param {import("../../api/plugins").PluginView} param0.plugin
 * @param {() => void} param0.onReload
 */
export default function PluginCard({ plugin, onReload }) {
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

  /** 打开插件的写入目录：交给系统文件管理器，失败只提示不改页面状态。 */
  async function openConfigDir(path) {
    try {
      await openPath(path);
    } catch (err) {
      // opener 是插件命令，失败不经 Rust 命令层，前端自行落盘（ADR 0008）。
      console.error(String(err));
      message.error(String(err));
    }
  }

  const title = (
    <Flex justify="space-between" align="center">
      <Typography.Text strong>{plugin.id || plugin.source}</Typography.Text>
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
        {plugin.config_dir && (
          <span>
            写入目录：{plugin.config_dir}{" "}
            <Tooltip title="打开文件夹">
              <Button
                size="small"
                shape="circle"
                aria-label="打开文件夹"
                icon={<FolderOpenOutlined />}
                onClick={() => openConfigDir(plugin.config_dir)}
              />
            </Tooltip>
          </span>
        )}
      </div>
      {plugin.error ? (
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
      <div className="card-actions">
        <Button
          disabled={busy}
          onClick={() =>
            run(() => reloadPlugin(plugin.source), "已重新加载插件")
          }
        >
          重新加载
        </Button>
        {!plugin.builtin && (
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
        )}
      </div>
    </Card>
  );
}
