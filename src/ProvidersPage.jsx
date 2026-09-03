import { useState } from "react";
import {
  Button,
  Card,
  Empty,
  Flex,
  Form,
  Input,
  Radio,
  Tag,
  Typography,
} from "antd";

const SLUG_PATTERN = /^[a-z][a-z0-9-_]*$/;

const PROTOCOL_OPTIONS = [
  {
    value: "openai-completions",
    label: "openai-completions（OpenAI 兼容 Chat Completions）",
  },
  {
    value: "anthropic-messages",
    label: "anthropic-messages（Anthropic Messages API）",
  },
];

export default function ProvidersPage() {
  // TODO(list_providers)：命令接入后由后端加载，并驱动加载/错误/空状态。
  // 当前为纯前端本地状态，仅用于预览布局；新建的 Provider 插入最前。
  const [providers, setProviders] = useState([]);
  const [creating, setCreating] = useState(false);
  const [form] = Form.useForm();

  function openCreate() {
    form.resetFields();
    setCreating(true);
  }

  function cancelCreate() {
    setCreating(false);
    form.resetFields();
  }

  // TODO(create_provider)：命令接入后改为调用后端并刷新列表。
  function handleSave(values) {
    setProviders((prev) => [
      {
        slug: values.slug,
        base_url: { [values.protocol]: values.baseUrl },
        api_key_set: false,
        model_count: 0,
      },
      ...prev,
    ]);
    setCreating(false);
    form.resetFields();
  }

  return (
    <div className="page">
      <div className="page-header">
        <Typography.Title level={4} style={{ margin: 0 }}>
          Provider
        </Typography.Title>
        <Button type="primary" disabled={creating} onClick={openCreate}>
          新建
        </Button>
      </div>

      {providers.length === 0 && !creating ? (
        <Empty description="尚未接入任何 Provider">
          <Button type="primary" onClick={openCreate}>
            新建 Provider
          </Button>
        </Empty>
      ) : (
        <Flex vertical gap={16}>
          {creating && (
            <Card title="新建 Provider">
              <Form form={form} layout="vertical" onFinish={handleSave}>
                <Form.Item
                  name="slug"
                  label="Slug"
                  rules={[
                    { required: true, message: "请输入 slug" },
                    {
                      pattern: SLUG_PATTERN,
                      message:
                        "slug 需以小写字母开头，仅含小写字母、数字、连字符或下划线",
                    },
                    {
                      validator: (_, value) =>
                        value && providers.some((p) => p.slug === value)
                          ? Promise.reject(
                              new Error(`已存在同名 Provider：${value}`),
                            )
                          : Promise.resolve(),
                    },
                  ]}
                >
                  <Input placeholder="例如 ollama" />
                </Form.Item>

                <Form.Item
                  name="protocol"
                  label="协议"
                  rules={[{ required: true, message: "请选择协议" }]}
                >
                  <Radio.Group
                    className="protocol-radios"
                    options={PROTOCOL_OPTIONS}
                  />
                </Form.Item>

                <Form.Item
                  name="baseUrl"
                  label="Base URL"
                  rules={[
                    { required: true, message: "请输入 Base URL" },
                    {
                      validator: (_, value) => {
                        if (!value) return Promise.resolve();
                        let url;
                        try {
                          url = new URL(value);
                        } catch {
                          return Promise.reject(
                            new Error("Base URL 不是合法的 URL"),
                          );
                        }
                        if (url.protocol !== "http:" && url.protocol !== "https:") {
                          return Promise.reject(
                            new Error("Base URL 仅支持 http(s) 协议"),
                          );
                        }
                        return Promise.resolve();
                      },
                    },
                  ]}
                >
                  <Input placeholder="例如 http://localhost:11434/v1" />
                </Form.Item>

                <div className="card-actions">
                  <Button onClick={cancelCreate}>取消</Button>
                  <Button type="primary" onClick={() => form.submit()}>
                    保存
                  </Button>
                </div>
              </Form>
            </Card>
          )}

          {providers.map((provider) => (
            <Card key={provider.slug} title={provider.slug}>
              <Form layout="vertical" disabled initialValues={provider}>
                {Object.entries(provider.base_url ?? {}).map(([protocol]) => (
                  <Form.Item
                    key={protocol}
                    name={["base_url", protocol]}
                    label={protocol}
                  >
                    <Input />
                  </Form.Item>
                ))}
              </Form>
              <div className="provider-meta">
                <span>
                  API Key <Tag>{provider.api_key_set ? "已设置" : "未设置"}</Tag>
                </span>
                <span>模型数：{provider.model_count}</span>
              </div>
            </Card>
          ))}
        </Flex>
      )}
    </div>
  );
}
