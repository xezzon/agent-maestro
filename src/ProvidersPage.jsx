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
  Radio,
  Spin,
  Switch,
  Tag,
  Tooltip,
  Typography,
} from "antd";
import { CheckCircleTwoTone, FileTextOutlined } from "@ant-design/icons";
import {
  ANTHROPIC_MESSAGES,
  OPENAI_COMPLETIONS,
  PROVIDER_PROTOCOLS,
  createProvider,
  deleteProvider,
  listProviders,
  normalizeEndpoints,
  setProviderEnabled,
  updateProvider,
} from "./api/provider";
import { applyProviders, listPlugins } from "./api/plugins";
import { listVariables } from "./api/variables";
import VariablesCard from "./components/VariablesCard";
import { openPath } from "@tauri-apps/plugin-opener";

/**
 * @param {Object} param0
 * @param {import("./api/provider").Provider} param0.provider
 * @param {() => Promise<void>} param0.onReload
 */
function ProviderCard({ provider, onReload }) {
  const [editing, setEditing] = useState(false);
  // 开关与编辑表单互斥：任一方进行中另一方整体禁用。否则切换成功后、
  // 列表刷新落地前提交的表单仍持有切换前的 provider，会把刚完成的禁用改回启用。
  const [busy, setBusy] = useState(false);

  /** 开关独立于编辑表单：切换失败保持原状态。 */
  async function handleToggleEnabled(enabled) {
    setBusy(true);
    try {
      await setProviderEnabled(provider.slug, enabled);
      await onReload();
    } catch (err) {
      message.error(String(err));
    } finally {
      setBusy(false);
    }
  }

  const title = (
    <Flex justify="space-between" align="center">
      <Flex align="center" gap={8}>
        <Typography.Text strong>{provider.slug}</Typography.Text>
        {!provider.enabled && <Tag>已禁用</Tag>}
      </Flex>
      <Switch
        checked={provider.enabled}
        checkedChildren="启用"
        unCheckedChildren="禁用"
        disabled={busy}
        onChange={handleToggleEnabled}
      />
    </Flex>
  );

  return <Card title={title} className={provider.enabled ? undefined : "provider-disabled"}>
    {editing
      ? <ProviderForm
        provider={provider}
        providers={[]} // 更新状态下不需要检查 slug 冲突（因为 slug 不可编辑）
        disabled={busy}
        onBusyChange={setBusy}
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
 * @param {import("./api/provider").Provider} param0.provider
 * @param {() => void} param0.afterDelete
 * @param {() => void} param0.onEdit
 */
function ProviderReadonlyForm({ provider, afterDelete, onEdit }) {
  const [deleting, setDeleting] = useState(false);

  async function handleDelete() {
    setDeleting(true);
    try {
      await deleteProvider(provider.slug);
      afterDelete();
    } catch (err) {
      message.error(String(err));
    } finally {
      setDeleting(false);
    }
  }

  return <>
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
      <span>模型数：{provider.models?.length ?? 0}</span>
    </div>
    {(provider.models?.length ?? 0) > 0 && (
      <ul className="provider-models">
        {provider.models.map((model, index) => (
          <li key={`${model.id}-${index}`}>
            <Typography.Text>{model.display_name || model.id}</Typography.Text>
            {model.display_name ? (
              <Typography.Text type="secondary">{model.id}</Typography.Text>
            ) : null}
          </li>
        ))}
      </ul>
    )}
    {(provider.custom_header?.length ?? 0) > 0 && (
      // 只列键名，绝不显示值：值是凭证（issue #58 决定 9）。
      <div className="provider-headers">
        <Typography.Text type="secondary">Header</Typography.Text>
        {provider.custom_header.map((header) => (
          <Tag key={header.name}>{header.name}</Tag>
        ))}
      </div>
    )}
    <div className="card-actions">
      <Button disabled={deleting} onClick={onEdit}>
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
        <Button danger disabled={deleting}>
          删除
        </Button>
      </Popconfirm>
    </div>
  </>
}

/**
 * @param {Object} param0
 * @param {import("./api/provider").Provider} param0.provider
 * @param {import("./api/provider").Provider[]} param0.providers 当前存在的 providers，用于检查 slug 冲突
 * @param {(refresh: boolean) => void} param0.onFinish
 * @param {boolean=} param0.disabled 外部（状态开关）进行中时整体禁用表单
 * @param {(busy: boolean) => void=} param0.onBusyChange 上报本表单的保存态
 */
function ProviderForm({ provider, providers, onFinish, disabled = false, onBusyChange }) {
  const SLUG_PATTERN = /^[a-z][a-z0-9-_]*$/;
  const PROTOCOL_OPTIONS = [
    {
      value: OPENAI_COMPLETIONS,
      label: "openai-completions（OpenAI 兼容 Chat Completions）",
    },
    {
      value: ANTHROPIC_MESSAGES,
      label: "anthropic-messages（Anthropic Messages API）",
    },
  ];
  const BASE_URL_RULES = [
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
  // 两个协议至少填一个端点：两个 URL 字段各自的 BASE_URL_RULES 都放行空值，
  // 这里在 openai 槽位上补一条整体校验（提交时全量 validateFields 会一并触发）。
  const AT_LEAST_ONE_ENDPOINT_RULE = {
    validator: () => {
      const endpoints = form.getFieldValue("base_url") ?? {};
      return PROVIDER_PROTOCOLS.some(
        (protocol) => (endpoints[protocol] ?? "").trim() !== "",
      )
        ? Promise.resolve()
        : Promise.reject(new Error("请至少填写一个协议的 Base URL"));
    },
  };
  // 两个协议都配置时必须选定投影使用的端点；只配置一个时无需选择。
  const SELECTED_PROTOCOL_RULES = [
    {
      validator: (_, value) => {
        const filled = PROVIDER_PROTOCOLS.filter(
          (protocol) =>
            (form.getFieldValue(["base_url", protocol]) ?? "").trim() !== "",
        );
        if (filled.length < 2) return Promise.resolve();
        return filled.includes(value)
          ? Promise.resolve()
          : Promise.reject(new Error("请选择投影使用的端点"));
      },
    },
  ];
  // HTTP token 字符集（RFC 9110 的 tchar）：header 名必须是合法 token，工具侧才能解析。
  const HEADER_NAME_PATTERN = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;
  // header 名非空、限 HTTP token 字符集、同一 Provider 内忽略大小写唯一。
  // 后端存大小写敏感的原名（issue #58 决定 11），这三条校验只在前端内联，不在 Rust 侧重复。
  const HEADER_NAME_RULES = [
    { required: true, message: "请输入 Header 名" },
    {
      pattern: HEADER_NAME_PATTERN,
      message: "Header 名仅允许字母、数字与 !#$%&'*+-.^_`|~",
    },
    {
      validator: (_, value) => {
        if (!value) return Promise.resolve();
        // 唯一性校验依赖当前表单内全部 header 行的实时值。
        const occurrences = (form.getFieldValue("custom_header") ?? []).filter(
          (header) => header?.name?.toLowerCase() === value.toLowerCase(),
        ).length;
        return occurrences > 1
          ? Promise.reject(
            new Error(`Header 名「${value}」在当前 Provider 内重复（忽略大小写）`),
          )
          : Promise.resolve();
      },
    },
  ];
  // 值可能含凭证，必填但不在前端展示明文（掩码输入）。
  const HEADER_VALUE_RULES = [{ required: true, message: "请输入 Header 值" }];
  // 模型 ID 非空（空白串视为空）且同一 Provider 内唯一（大小写敏感）；
  // 唯一性校验依赖当前表单内全部模型行的实时值。
  const MODEL_ID_RULES = [
    {
      validator: (_, value) => {
        const id = value ?? "";
        if (!id.trim()) {
          return Promise.reject(new Error("请输入模型 ID"));
        }
        const occurrences = (form.getFieldValue("models") ?? []).filter(
          (model) => model?.id === value,
        ).length;
        return occurrences > 1
          ? Promise.reject(new Error(`模型 ID「${value}」在当前 Provider 内重复`))
          : Promise.resolve();
      },
    },
  ];

  const [form] = Form.useForm();
  // 实时追踪两个端点槽位，用于禁用「未配置」协议的投影选项。
  const watchedEndpoints = Form.useWatch("base_url", form) ?? {};
  const filledProtocols = PROVIDER_PROTOCOLS.filter(
    (protocol) => (watchedEndpoints[protocol] ?? "").trim() !== "",
  );
  const [saving, setSaving] = useState(false);

  function handleSubmit() {
    submitWith(provider.slug ? updateProvider : createProvider);
  }

  // 命令成功即已落盘，由父组件刷新列表。
  // `enabled` 不是 Form.Item（开关独立于表单），validateFields 不返回它；
  // 显式与 provider 合并回传，避免缺字段被后端默认值静默重置为启用。
  async function submitWith(apiFn) {
    setSaving(true);
    onBusyChange?.(true);
    try {
      const values = await form.validateFields();
      await apiFn({ ...provider, ...values });
      onFinish(true);
    } catch (err) {
      // 校验失败已内联展示，无需重复报错；其余为命令调用失败。
      if (!err?.errorFields) {
        message.error(String(err));
      }
    } finally {
      setSaving(false);
      onBusyChange?.(false);
    }
  }

  return <Form
    form={form}
    layout="vertical"
    disabled={disabled}
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
      name={["base_url", OPENAI_COMPLETIONS]}
      label={`Base URL（${OPENAI_COMPLETIONS}）`}
      rules={[...BASE_URL_RULES, AT_LEAST_ONE_ENDPOINT_RULE]}
    >
      <Input placeholder="例如 http://localhost:11434/v1" />
    </Form.Item>

    <Form.Item
      name={["base_url", ANTHROPIC_MESSAGES]}
      label={`Base URL（${ANTHROPIC_MESSAGES}）`}
      rules={BASE_URL_RULES}
    >
      <Input placeholder="例如 https://api.anthropic.com" />
    </Form.Item>

    <Form.Item
      name="selected_protocol"
      label="投影使用"
      rules={SELECTED_PROTOCOL_RULES}
      extra="只配置一个协议时无需选择；两个都配置时必选，投影按所选协议写入。"
    >
      <Radio.Group
        className="protocol-radios"
        options={PROTOCOL_OPTIONS.map((option) => ({
          ...option,
          disabled: !filledProtocols.includes(option.value),
        }))}
      />
    </Form.Item>

    <Form.Item
      name="api_key"
      label="API Key"
      extra="可选；本地网关可留空。以明文保存在本地配置文件中"
    >
      <Input.Password
        placeholder="留空则不设置"
        autoComplete="new-password"
        visibilityToggle={false}
      />
    </Form.Item>

    <Form.Item
      label="自定义 Header"
      extra="可选；随该 Provider 下所有模型与协议共享。值以明文保存在本地配置文件中"
    >
      <Form.List name="custom_header">
        {(fields, { add, remove }) => (
          <div className="header-rows">
            {fields.map((field) => (
              <Flex key={field.key} align="flex-start" gap={8}>
                <Form.Item
                  name={[field.name, "name"]}
                  rules={HEADER_NAME_RULES}
                  className="header-field"
                >
                  <Input placeholder="Header 名，例如 anthropic-version" />
                </Form.Item>
                <Form.Item
                  name={[field.name, "value"]}
                  rules={HEADER_VALUE_RULES}
                  className="header-field"
                >
                  <Input.Password
                    placeholder="Header 值"
                    autoComplete="new-password"
                    visibilityToggle={false}
                  />
                </Form.Item>
                <Button disabled={saving} onClick={() => remove(field.name)}>
                  删除
                </Button>
              </Flex>
            ))}
            <Button type="dashed" block disabled={saving} onClick={() => add()}>
              添加 Header
            </Button>
          </div>
        )}
      </Form.List>
    </Form.Item>

    <Form.Item label="模型">
      <Form.List name="models">
        {(fields, { add, remove }) => (
          <div className="model-rows">
            {fields.map((field) => (
              <Flex key={field.key} align="flex-start" gap={8} className="model-row">
                <Form.Item
                  name={[field.name, "id"]}
                  rules={MODEL_ID_RULES}
                  className="model-field"
                >
                  <Input placeholder="模型 ID，例如 gpt-4o" />
                </Form.Item>
                <Form.Item
                  name={[field.name, "display_name"]}
                  className="model-field"
                >
                  <Input placeholder="显示名（可选，留空回退显示模型 ID）" />
                </Form.Item>
                <Button disabled={saving} onClick={() => remove(field.name)}>
                  删除
                </Button>
              </Flex>
            ))}
            <Button type="dashed" block disabled={saving} onClick={() => add()}>
              添加模型
            </Button>
          </div>
        )}
      </Form.List>
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

/**
 * 逐插件投影结果（见 issue #34 用户故事 2/3/4）。
 * @param {Object} param0
 * @param {import("./api/plugins").PluginApplyReport[]} param0.reports
 */
function ApplyResultList({ reports }) {
  return (
    <Flex vertical gap={12}>
      {reports.map((report) => (
        <div key={report.id}>
          <Flex align="center" gap={8}>
            <Typography.Text strong>{report.id}</Typography.Text>
            {report.status === "applied" && (
              <Tag color="success">已写入</Tag>
            )}
            {report.status === "failed" && <Tag color="error">失败</Tag>}
            {report.status === "skipped" && <Tag>已跳过</Tag>}
          </Flex>
          {report.files.length > 0 && (
            <Flex align="center" gap={8}>
              <Typography.Paragraph type="secondary" style={{ margin: 0 }}>
                写入文件：{report.files.join("、")}
              </Typography.Paragraph>
              <Tooltip title="打开文件">
                <Button
                  size="small"
                  shape="circle"
                  aria-label="打开文件"
                  icon={<FileTextOutlined />}
                  onClick={async () => {
                    try {
                      await openPath(report.files[0]);
                    } catch (err) {
                      // opener 是插件命令，失败不经 Rust 命令层，前端自行落盘（ADR 0008）。
                      console.error(String(err));
                      message.error(String(err));
                    }
                  }}
                />
              </Tooltip>
            </Flex>
          )}
          {report.skipped.map((skip) => (
            <Typography.Paragraph type="secondary" style={{ margin: 0 }} key={skip.slug}>
              跳过 Provider「{skip.slug}」：{skip.reason}
            </Typography.Paragraph>
          ))}
          {report.reason && (
            <Typography.Paragraph type="danger" style={{ margin: 0 }}>
              {report.reason}
            </Typography.Paragraph>
          )}
        </div>
      ))}
    </Flex>
  );
}

export default function ProvidersPage() {
  const [providers, setProviders] = useState([]);
  const [variables, setVariables] = useState({});
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(null);
  const [creating, setCreating] = useState(false);
  const [applying, setApplying] = useState(false);
  const [canApply, setCanApply] = useState(false);
  const [reports, setReports] = useState(null);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const [providers, variables] = await Promise.all([listProviders(), listVariables()]);
      setProviders(providers);
      setVariables(variables);
      setLoadError(null);
    } catch (err) {
      setLoadError(String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    reload();
    // 没有可用插件（已启用且加载成功）时投影按钮禁用（用户故事 24）。
    listPlugins()
      .then((plugins) =>
        setCanApply(plugins.some((p) => p.enabled && !p.error)),
      )
      .catch(() => setCanApply(false));
  }, [reload]);

  async function handleApply() {
    setApplying(true);
    try {
      setReports(await applyProviders());
    } catch (err) {
      message.error(String(err));
    } finally {
      setApplying(false);
    }
  }

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
    <div className="page providers-page">
      <div className="page-header">
        <Typography.Title level={4} style={{ margin: 0 }}>
          Provider
        </Typography.Title>
        <Flex gap={8}>
          <Button
            disabled={!canApply || creating}
            loading={applying}
            onClick={handleApply}
          >
            应用到工具
          </Button>
          <Button type="primary" disabled={creating || loading} onClick={openCreate}>
            新建
          </Button>
        </Flex>
      </div>

      <Modal
        title="应用到工具"
        open={reports !== null}
        footer={
          <Button type="primary" onClick={() => setReports(null)}>
            关闭
          </Button>
        }
        onCancel={() => setReports(null)}
      >
        <ApplyResultList reports={reports ?? []} />
      </Modal>

      <div className="providers-layout">
        <div className="providers-main">
          {loading && providers.length === 0 ? (
            <div className="page-loading">
              <Spin />
            </div>
          ) : (
            <Flex vertical gap={16}>
              {providers.length === 0 && !creating && (
                <Empty description="尚未接入任何 Provider">
                  <Button type="primary" onClick={openCreate}>
                    新建 Provider
                  </Button>
                </Empty>
              )}
              {
                creating && (
                  <Card title="新建 Provider">
                    <ProviderForm
                      provider={{
                        slug: "",
                        base_url: normalizeEndpoints(),
                        selected_protocol: null,
                        models: [],
                        enabled: true,
                      }}
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
                <ProviderCard key={provider.slug} provider={provider} onReload={reload} />
              )}
            </Flex>
          )}
        </div>
        <aside className="variables-sider">
          <VariablesCard variables={variables} onReload={reload} />
        </aside>
      </div>
    </div>
  );
}
