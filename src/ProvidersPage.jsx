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
  Radio,
  Spin,
  Tag,
  Typography,
} from "antd";
import { invoke } from "@tauri-apps/api/core";

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

// 后端直接返回以 slug 为 key 的 providers 原始结构；类型映射在 JS 侧完成。
function toProviderList(providersByKey) {
  return Object.entries(providersByKey ?? {}).map(([slug, provider]) => ({
    slug,
    base_url: provider.base_url ?? {},
    api_key_set: (provider.api_key ?? "") !== "",
    model_count: provider.models?.length ?? 0,
  }));
}

export default function ProvidersPage() {
  const [providers, setProviders] = useState([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(null);
  const [creating, setCreating] = useState(false);
  const [saving, setSaving] = useState(false);
  const [form] = Form.useForm();

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      setProviders(toProviderList(await invoke("list_providers")));
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

  function openCreate() {
    setCreating(true);
  }

  function cancelCreate() {
    setCreating(false);
  }

  // 命令成功即已落盘；用已知数据置顶插入，避免整表重载。重启后仍回 slug 序（version:1 无创建时间字段）。
  async function handleSave(values) {
    setSaving(true);
    try {
      await invoke("create_provider", {
        slug: values.slug,
        protocol: values.protocol,
        baseUrl: values.baseUrl,
      });
      const created = {
        [values.slug]: {
          base_url: { [values.protocol]: values.baseUrl },
          api_key: "",
          models: [],
        },
      };
      setProviders((prev) => [...toProviderList(created), ...prev]);
      setCreating(false);
    } catch (err) {
      message.error(String(err));
    } finally {
      setSaving(false);
    }
  }

  if (loadError) {
    return (
      <Alert
        type="error"
        showIcon
        message="配置加载失败"
        description={<pre className="error-detail">{loadError}</pre>}
      />
    );
  }

  return (
    <div className="page">
      <div className="page-header">
        <Typography.Title level={4} style={{ margin: 0 }}>
          Provider
        </Typography.Title>
        <Button type="primary" disabled={creating || loading} onClick={openCreate}>
          新建
        </Button>
      </div>

      {loading && providers.length === 0 ? (
        <div className="page-loading">
          <Spin />
        </div>
      ) : providers.length === 0 && !creating ? (
        <Empty description="尚未接入任何 Provider">
          <Button type="primary" onClick={openCreate}>
            新建 Provider
          </Button>
        </Empty>
      ) : (
        <Flex vertical gap={16}>
          {creating && (
            <Card title="新建 Provider">
              <Form
                form={form}
                layout="vertical"
                preserve={false}
                onFinish={handleSave}
              >
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
                            new Error("Base URL 仅支持 http(s) 地址"),
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
                  <Button disabled={saving} onClick={cancelCreate}>
                    取消
                  </Button>
                  <Button type="primary" loading={saving} onClick={() => form.submit()}>
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
