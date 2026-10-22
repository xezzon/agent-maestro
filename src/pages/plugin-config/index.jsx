import { useCallback, useEffect, useState } from "react";
import { Alert, Empty, Spin, Tabs, Typography } from "antd";
import { getPluginConfig } from "../../api/plugins";
import { listVariables } from "../../api/variables";
import PluginVariablesCard from "./PluginVariablesCard";
import PluginForm from "./PluginForm";

/**
 * 插件配置页：分「变量覆盖」「表单」两个 Tab。表单按 manifest 的
 * settings_schema（ADR 0017）渲染，写入插件配置的 form 段。
 * 保存只写该插件的配置（ADR 0018），不触发投影——投影仍由 Provider 页的
 * 「应用到工具」显式发起。
 * @param {Object} param0
 * @param {import("../../api/plugins").PluginView} param0.plugin
 */
export default function PluginConfigPage({ plugin }) {
  const [config, setConfig] = useState(null);
  // 全局变量表（ADR 0015）：覆盖项的名字校验依赖它（声明的真相源）。
  const [variables, setVariables] = useState({});
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(null);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const [nextConfig, nextVariables] = await Promise.all([
        getPluginConfig(plugin.source),
        listVariables(),
      ]);
      setConfig(nextConfig);
      setVariables(nextVariables);
      setLoadError(null);
    } catch (err) {
      setLoadError(String(err));
    } finally {
      setLoading(false);
    }
  }, [plugin.source]);

  useEffect(() => {
    reload();
  }, [reload]);

  if (loadError) {
    return (
      <div className="page">
        <PageHeader plugin={plugin} />
        <Alert
          type="error"
          showIcon
          title="插件配置加载失败"
          description={<pre className="error-detail">{loadError}</pre>}
        />
      </div>
    );
  }

  return (
    <div className="page">
      <PageHeader plugin={plugin} />
      {loading && config === null ? (
        <div className="page-loading">
          <Spin />
        </div>
      ) : (
        <Tabs
          defaultActiveKey="variables"
          items={[
            {
              key: "variables",
              label: "变量覆盖",
              children: (
                <PluginVariablesCard
                  plugin={plugin}
                  config={config}
                  variables={variables}
                  onReload={reload}
                />
              ),
            },
            {
              key: "form",
              label: "表单",
              children: plugin.settings_schema ? (
                <PluginForm plugin={plugin} config={config} onReload={reload} />
              ) : (
                <Empty description="该插件没有需要填写的配置项" />
              ),
            },
          ]}
        />
      )}
    </div>
  );
}

/**
 * 页头：插件标识（加载失败与正常两个分支共用）。
 * @param {Object} param0
 * @param {import("../../api/plugins").PluginView} param0.plugin
 */
function PageHeader({ plugin }) {
  return (
    <div className="page-header">
      <Typography.Title level={4} style={{ margin: 0 }}>
        {plugin.id || plugin.source}
      </Typography.Title>
    </div>
  );
}
