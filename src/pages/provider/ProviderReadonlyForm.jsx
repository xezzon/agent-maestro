import {
  Badge,
  Button,
  Flex,
  Form,
  Input,
  Popconfirm,
  Tag,
  Tooltip,
  Typography,
  theme,
} from "antd";
import { CheckCircleTwoTone } from "@ant-design/icons";
import {
  CAPABILITY_LABELS,
  PROVIDER_PROTOCOLS,
  deleteProvider,
} from "@/api/provider";
import useAsyncAction from "@/hooks/useAsyncAction";

/** 只读展示一项 token 上限：未设置就显示「未设置」，不编造默认值。 */
function formatLimit(value) {
  return value == null ? "未设置" : String(value);
}

/**
 * @param {Object} param0
 * @param {import("@/api/provider").Provider} param0.provider
 * @param {() => void} param0.afterDelete
 * @param {() => void} param0.onEdit
 */
export default function ProviderReadonlyForm({
  provider,
  afterDelete,
  onEdit,
}) {
  const [busy, run] = useAsyncAction();
  const { useToken } = theme;
  const { token } = useToken();

  async function handleDelete() {
    await run(async () => {
      await deleteProvider(provider.slug);
      afterDelete();
    });
  }

  return (
    <>
      <Form layout="vertical" disabled>
        {PROVIDER_PROTOCOLS.map((protocol) => (
          <Form.Item
            key={protocol}
            label={
              <Flex align="center" gap={8}>
                <span>{protocol}</span>
                {provider.selected_protocol === protocol && (
                  <Tooltip title="已选择">
                    <Tag
                      color="blue"
                      icon={<CheckCircleTwoTone aria-label="已选择" />}
                    />
                  </Tooltip>
                )}
              </Flex>
            }
          >
            <Input
              disabled
              value={provider.base_url?.[protocol] ?? ""}
              placeholder="未配置"
            />
          </Form.Item>
        ))}
      </Form>
      <div className="provider-meta">
        <span>
          API Key <Tag>{provider.api_key_set ? "已设置" : "未设置"}</Tag>
        </span>
      </div>
      {(provider.custom_header?.length ?? 0) > 0 && (
        // 只列键名，绝不显示值：值是凭证（issue #58 决定 9）。
        <div className="provider-headers">
          <Typography.Text>Header</Typography.Text>
          {provider.custom_header.map((header) => (
            <Tag key={header.name}>{header.name}</Tag>
          ))}
        </div>
      )}
      <Flex align="center" gap={8} className="provider-models-title">
        <Typography.Title level={3} style={{ margin: 0 }}>
          模型
        </Typography.Title>
        <Badge
          count={provider.models?.length ?? 0}
          showZero
          color={token.colorPrimary}
        />
      </Flex>
      {(provider.models?.length ?? 0) > 0 && (
        <ul className="provider-models">
          {provider.models.map((model, index) => (
            <li key={`${model.id}-${index}`}>
              <Flex vertical gap={4}>
                <Flex align="baseline" gap={8}>
                  <Typography.Text>
                    {model.display_name || model.id}
                  </Typography.Text>
                  {model.display_name ? (
                    <Typography.Text type="secondary">
                      {model.id}
                    </Typography.Text>
                  ) : null}
                </Flex>
                {(model.capabilities?.length ?? 0) > 0 ? (
                  <Flex align="center" gap={4}>
                    {model.capabilities.map((capability) => (
                      <Tag key={capability}>
                        {CAPABILITY_LABELS[capability]}
                      </Tag>
                    ))}
                  </Flex>
                ) : null}
                <Typography.Text type="secondary">
                  上下文窗口 {formatLimit(model.limit?.context_window)}
                  ｜单次最大输入 {formatLimit(model.limit?.max_input)}
                  ｜最大输出 {formatLimit(model.limit?.max_output)}
                </Typography.Text>
              </Flex>
            </li>
          ))}
        </ul>
      )}
      <div className="card-actions">
        <Button disabled={busy} onClick={onEdit}>
          编辑
        </Button>
        <Popconfirm
          title="删除 Provider"
          description={`将同时删除「${provider.slug}」的模型与 API Key，确定删除？`}
          okText="删除"
          cancelText="取消"
          okButtonProps={{ danger: true }}
          onConfirm={handleDelete}
        >
          <Button danger disabled={busy}>
            删除
          </Button>
        </Popconfirm>
      </div>
    </>
  );
}
