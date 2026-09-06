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
  Radio,
  Spin,
  Tag,
  Typography,
} from "antd";
import { createProvider, deleteProvider, listProviders, updateProvider } from "./api/provider";


const OPENAI_COMPLTIONS = "openai-completions";
const ANTHROPIC_MESSAGES = "anthropic-messages";

/**
 * @param {Object} param0 
 * @param {import("./api/provider").ProviderFormData} param0.provider
 * @param {() => void} param0.onReload
 */
function ProviderCard({ provider, onReload }) {
  const [editing, setEditing] = useState(false);

  return <Card title={provider.slug}>
    {editing
      ? <ProviderForm
        provider={provider}
        providers={[]} // 更新状态下不需要检查 slug 冲突（因为 slug 不可编辑）
        onFinish={(refresh) => {
          setEditing(false)
          if (refresh) {
            onReload();
          }
        }}
      />
      : <ProviderReadonlyForm
        provider={provider}
        onEdit={() => setEditing(true)}
        afterDelete={onReload}
      />
    }
  </Card>
}

/**
 * @param {Object} param0 
 * @param {import("./api/provider").ProviderFormData} param0.provider
 * @param {() => void} param0.afterDelete
 * @param {() => void} param0.onEdit
 */
function ProviderReadonlyForm({ provider, afterDelete, onEdit }) {
  const [deleting, setDeleting] = useState(false);

  // 返回的 warnings 为「删除成功但密钥链清除失败」的降级提示。
  async function handleDelete() {
    setDeleting(true);
    try {
      const warnings = await deleteProvider(provider.slug);
      afterDelete();
      (warnings ?? []).forEach((warning) => message.warning(warning));
    } catch (err) {
      message.error(String(err));
    } finally {
      setDeleting(false);
    }
  }

  return <>
    <Form layout="vertical" disabled>
      <Form.Item label={provider.protocol}>
        <Input disabled value={provider.base_url} />
      </Form.Item>
    </Form>
    <div className="provider-meta">
      <span>
        API Key <Tag>{provider.api_key ? "已设置" : "未设置"}</Tag>
      </span>
      <span>模型数：{provider.models?.length ?? 0}</span>
    </div>
    <div className="card-actions">
      <Button disabled={deleting} onClick={onEdit}>
        编辑
      </Button>
      <Popconfirm
        title="删除 Provider"
        description={`将同时清除「${provider.slug}」的模型与密钥链条目，确定删除？`}
        okText="删除"
        cancelText="取消"
        okButtonProps={{ danger: true }}
        onConfirm={handleDelete}
      >
        <Button danger disabled={deleting}>
          删除
        </Button>
      </Popconfirm>
    </div>
  </>
}

/**
 * @param {Object} param0 
 * @param {import("./api/provider").ProviderFormData} param0.provider
 * @param {import("./api/provider").ProviderFormData[]} param0.providers 当前存在的 providers，用于检查 slug 冲突
 * @param {(refresh: boolean) => void} param0.onFinish
 */
function ProviderForm({ provider, providers, onFinish }) {
  const SLUG_PATTERN = /^[a-z][a-z0-9-_]*$/;
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
  const PROTOCOL_OPTIONS = [
    {
      value: OPENAI_COMPLTIONS,
      label: "openai-completions（OpenAI 兼容 Chat Completions）",
    },
    {
      value: ANTHROPIC_MESSAGES,
      label: "anthropic-messages（Anthropic Messages API）",
    },
  ];

  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);

  function handleSubmit() {
    const submit = provider.slug ? handleUpdate : handleSave;
    submit();
  }

  // 命令成功即已落盘；用已知数据置顶插入，避免整表重载。重启后仍回 slug 序（version:1 无创建时间字段）。
  async function handleSave() {
    setSaving(true);
    try {
      await form.validateFields()
        .then(createProvider)
        .then(() => onFinish(true));
    } catch (err) {
      message.error(String(err));
    } finally {
      setSaving(false);
    }
  }

  // 命令成功即已落盘；就地更新已知数据，避免整表重载。
  async function handleUpdate() {
    setSaving(true);
    try {
      await form.validateFields()
        .then(updateProvider)
        .then(() => onFinish(true));
    } catch (err) {
      message.error(String(err));
    } finally {
      setSaving(false);
    }
  }

  return <Form
    form={form}
    layout="vertical"
    preserve={false}
    initialValues={provider}
    onFinish={handleSubmit}
  >
    <Form.Item
      name="slug"
      label="Slug"
      extra="slug 创建后不可修改"
      rules={[
        { required: true, message: "请输入 slug" },
        {
          pattern: SLUG_PATTERN,
          message:
            "slug 需以小写字母开头，仅允许小写字母、数字、连字符和下划线",
        },
        {
          validator: (_, value) =>
            value !== provider.slug && providers.some((p) => p.slug === value)
              ? Promise.reject(
                new Error(`slug ${value} 已被使用`),
              )
              : Promise.resolve(),
        },
      ]}
    >
      <Input disabled={!!provider.slug} />
    </Form.Item>

    <Form.Item
      name="protocol"
      label="协议"
      rules={[{ required: true, message: "请选择协议" }]}
    >
      <Radio.Group className="protocol-radios" options={PROTOCOL_OPTIONS} />
    </Form.Item>

    <Form.Item name="base_url" label="Base URL" rules={BASE_URL_RULES}>
      <Input placeholder="例如 http://localhost:11434/v1" />
    </Form.Item>

    <div className="card-actions">
      <Button disabled={saving} onClick={() => onFinish(false)}>
        取消
      </Button>
      <Button
        type="primary"
        loading={saving}
        onClick={() => form.submit()}
      >
        保存
      </Button>
    </div>
  </Form>
}

export default function ProvidersPage() {
  const [providers, setProviders] = useState([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(null);
  const [creating, setCreating] = useState(false);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const providers = await listProviders()
      setProviders(providers);
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

  if (loadError) {
    return (
      <Alert
        type="error"
        showIcon
        title="配置加载失败"
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
          {
            creating && (
              <Card title="新建 Provider">
                <ProviderForm
                  provider={{ slug: "", protocol: null, base_url: "" }}
                  providers={providers}
                  onFinish={(refresh) => {
                    setCreating(false);
                    if (refresh) {
                      reload();
                    }
                  }}
                />
              </Card>
            )
          }
          {providers.map((provider) =>
            <ProviderCard provider={provider} onReload={reload} />
          )}
        </Flex>
      )}
    </div>
  );
}
