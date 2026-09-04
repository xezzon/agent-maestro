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

const BASE_URL_RULES = [
  { required: true, message: "请输入 Base URL" },
  {
    validator: (_, value) => {
      if (!value) return Promise.resolve();
      let url;
      try {
        url = new URL(value);
      } catch {
        return Promise.reject(new Error("Base URL 不是合法的 URL"));
      }
      if (url.protocol !== "http:" && url.protocol !== "https:") {
        return Promise.reject(new Error("Base URL 仅支持 http(s) 地址"));
      }
      return Promise.resolve();
    },
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

// 新建/编辑共用的协议与端点字段（表单字段名一致：protocol、baseUrl）。
function EndpointFields() {
  return (
    <>
      <Form.Item
        name="protocol"
        label="协议"
        rules={[{ required: true, message: "请选择协议" }]}
      >
        <Radio.Group className="protocol-radios" options={PROTOCOL_OPTIONS} />
      </Form.Item>

      <Form.Item name="baseUrl" label="Base URL" rules={BASE_URL_RULES}>
        <Input placeholder="例如 http://localhost:11434/v1" />
      </Form.Item>
    </>
  );
}

export default function ProvidersPage() {
  const [providers, setProviders] = useState([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(null);
  const [creating, setCreating] = useState(false);
  const [saving, setSaving] = useState(false);
  // 编辑中的 Provider：{ slug, protocol（当前单选的协议）, slots（各协议槽位的当前值） }
  const [editing, setEditing] = useState(null);
  const [form] = Form.useForm();
  const [editForm] = Form.useForm();

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

  function openEdit(provider) {
    const slots = { ...provider.base_url };
    // 默认选中已配置的协议；均未配置时回退到第一项
    const configured = PROTOCOL_OPTIONS.find((option) => slots[option.value]);
    const protocol = configured ? configured.value : PROTOCOL_OPTIONS[0].value;
    setEditing({ slug: provider.slug, protocol, slots });
  }

  function cancelEdit() {
    setEditing(null);
  }

  // Base URL 输入框绑定当前所选协议槽位：切换协议时换到对应槽位的值
  function handleEditValuesChange(changed) {
    if (changed.protocol !== undefined) {
      editForm.setFieldValue("baseUrl", editing.slots[changed.protocol] ?? "");
      setEditing((prev) => ({ ...prev, protocol: changed.protocol }));
    } else if (changed.baseUrl !== undefined) {
      setEditing((prev) => ({
        ...prev,
        slots: { ...prev.slots, [prev.protocol]: changed.baseUrl },
      }));
    }
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

  // 命令成功即已落盘；就地更新已知数据，避免整表重载。
  async function handleUpdate(values) {
    setSaving(true);
    try {
      await invoke("update_provider", {
        slug: editing.slug,
        protocol: values.protocol,
        baseUrl: values.baseUrl,
      });
      setProviders((prev) =>
        prev.map((provider) =>
          provider.slug === editing.slug
            ? {
                ...provider,
                base_url: {
                  ...provider.base_url,
                  [values.protocol]: values.baseUrl,
                },
              }
            : provider,
        ),
      );
      setEditing(null);
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
        <Button type="primary" disabled={creating || !!editing || loading} onClick={openCreate}>
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

                <EndpointFields />

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

          {editing && (
            <Card title={`编辑 Provider：${editing.slug}`}>
              <Form
                form={editForm}
                layout="vertical"
                preserve={false}
                initialValues={{
                  protocol: editing.protocol,
                  baseUrl: editing.slots[editing.protocol] ?? "",
                }}
                onValuesChange={handleEditValuesChange}
                onFinish={handleUpdate}
              >
                <Form.Item label="Slug" extra="slug 创建后不可修改">
                  <Input value={editing.slug} disabled />
                </Form.Item>

                <EndpointFields />

                <div className="card-actions">
                  <Button disabled={saving} onClick={cancelEdit}>
                    取消
                  </Button>
                  <Button
                    type="primary"
                    loading={saving}
                    onClick={() => editForm.submit()}
                  >
                    保存
                  </Button>
                </div>
              </Form>
            </Card>
          )}

          {providers.map((provider) => (
            <Card
              key={provider.slug}
              title={provider.slug}
              extra={
                <Button
                  size="small"
                  disabled={creating || !!editing || loading}
                  onClick={() => openEdit(provider)}
                >
                  编辑
                </Button>
              }
            >
              {/* 只读展示用受控输入（非表单状态），数据变化即时反映 */}
              <Form layout="vertical" disabled>
                {Object.entries(provider.base_url ?? {}).map(([protocol, url]) => (
                  <Form.Item key={protocol} label={protocol}>
                    <Input disabled value={url} />
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
