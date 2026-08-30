import { useCallback, useEffect, useState } from "react";
import {
  deletePlugin,
  deleteProvider,
  getConfig,
  savePlugin,
  saveProvider,
} from "./lib/api";
import PluginForm from "./components/PluginForm";
import PluginList from "./components/PluginList";
import ProviderForm from "./components/ProviderForm";
import ProviderList from "./components/ProviderList";
import "./App.css";

function App() {
  const [config, setConfig] = useState(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);
  const [providerForm, setProviderForm] = useState(null); // { provider } | null
  const [pluginForm, setPluginForm] = useState(null); // { plugin } | null
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setConfig(await getConfig());
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // Runs a mutation; backend errors surface in the banner and leave any open
  // form untouched so the user can correct and retry.
  const mutate = async (fn) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleSaveProvider = async (input) => {
    await mutate(async () => {
      setConfig(await saveProvider(input));
      setProviderForm(null);
    });
  };

  const handleDeleteProvider = async (provider) => {
    if (
      !window.confirm(
        `确定删除 Provider「${provider.id}」？其保存的密钥也会从系统密钥链中删除。`,
      )
    ) {
      return;
    }
    await mutate(async () => {
      setConfig(await deleteProvider(provider.id));
    });
  };

  const handleSavePlugin = async (input) => {
    await mutate(async () => {
      setConfig(await savePlugin(input));
      setPluginForm(null);
    });
  };

  const handleTogglePlugin = async (plugin) => {
    await mutate(async () => {
      setConfig(await savePlugin({ ...plugin, enabled: !plugin.enabled }));
    });
  };

  const handleDeletePlugin = async (plugin) => {
    if (!window.confirm(`确定删除 Plugin「${plugin.id}」？`)) {
      return;
    }
    await mutate(async () => {
      setConfig(await deletePlugin(plugin.id));
    });
  };

  return (
    <main className="app">
      <header className="app-header">
        <h1>Agent Maestro</h1>
        <p className="subtitle">Agent 工具的统一配置中心 · Provider / Plugin / 密钥</p>
      </header>

      {error && (
        <div className="error-banner" role="alert">
          {error}
        </div>
      )}

      {loading ? (
        <p className="empty-hint">加载中…</p>
      ) : (
        <>
          <section className="panel">
            <div className="panel-header">
              <h2>Provider</h2>
              <button
                type="button"
                className="primary-button"
                disabled={busy}
                onClick={() => setProviderForm({ provider: null })}
              >
                添加 Provider
              </button>
            </div>
            <ProviderList
              providers={config?.providers ?? []}
              onEdit={(provider) => setProviderForm({ provider })}
              onDelete={handleDeleteProvider}
            />
          </section>

          <section className="panel">
            <div className="panel-header">
              <h2>Plugin</h2>
              <button
                type="button"
                className="primary-button"
                disabled={busy}
                onClick={() => setPluginForm({ plugin: null })}
              >
                添加 Plugin
              </button>
            </div>
            <PluginList
              plugins={config?.plugins ?? []}
              onToggle={handleTogglePlugin}
              onEdit={(plugin) => setPluginForm({ plugin })}
              onDelete={handleDeletePlugin}
            />
          </section>
        </>
      )}

      {providerForm && (
        <ProviderForm
          initial={providerForm.provider}
          submitting={busy}
          onSubmit={handleSaveProvider}
          onCancel={() => setProviderForm(null)}
        />
      )}

      {pluginForm && (
        <PluginForm
          initial={pluginForm.plugin}
          submitting={busy}
          onSubmit={handleSavePlugin}
          onCancel={() => setPluginForm(null)}
        />
      )}
    </main>
  );
}

export default App;
