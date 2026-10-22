import { useCallback, useEffect, useState } from "react";
import {
  Alert,
  Button,
  Card,
  Empty,
  Flex,
  message,
  Modal,
  Spin,
  Typography,
} from "antd";
import { listProviders, normalizeEndpoints } from "../../api/provider";
import { applyProviders, listPlugins } from "../../api/plugins";
import { listVariables } from "../../api/variables";
import VariablesCard from "../../components/VariablesCard";
import ProviderCard from "./ProviderCard";
import ProviderForm from "./ProviderForm";
import { MODE_CREATE } from "./formMode";
import ApplyResultList from "./ApplyResultList";

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
                      mode={MODE_CREATE}
                      provider={{
                        slug: "",
                        base_url: normalizeEndpoints(),
                        selected_protocol: null,
                        models: [],
                        enabled: true,
                        api_key_set: false,
                      }}
                      providers={providers}
                      onCancel={() => setCreating(false)}
                      onSaved={() => {
                        setCreating(false);
                        reload();
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
