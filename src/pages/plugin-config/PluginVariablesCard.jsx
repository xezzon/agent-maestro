import { useState } from "react";
import { Button, Card } from "antd";
import VariableList from "@/components/VariableList";
import PluginVariablesForm from "./PluginVariablesForm";

/**
 * 变量覆盖卡片：该插件对全局变量的覆盖（名 → 覆盖值，ADR 0018）。
 * 数据由所在页面持有（config + 全局变量表 variables + onReload）。
 * 只读态展示覆盖项；编辑态按名增删改，保存整包写回插件配置（form 段原样带回）。
 * @param {Object} param0
 * @param {import("@/api/plugins").PluginView} param0.plugin
 * @param {import("@/api/plugins").PluginConfig} param0.config
 * @param {import("@/api/variables").Variables} param0.variables
 * @param {() => void} param0.onReload
 */
export default function PluginVariablesCard({
  plugin,
  config,
  variables,
  onReload,
}) {
  const [editing, setEditing] = useState(false);

  return (
    <Card title="变量覆盖">
      {editing ? (
        <PluginVariablesForm
          plugin={plugin}
          config={config}
          variables={variables}
          onSaved={() => {
            setEditing(false);
            onReload();
          }}
          onCancel={() => setEditing(false)}
        />
      ) : (
        <>
          <VariableList
            values={config.variables ?? {}}
            emptyText="尚未覆盖任何变量。覆盖后投影该插件时，覆盖值替代全局变量表中的默认值"
          />
          <div className="card-actions">
            <Button onClick={() => setEditing(true)}>编辑</Button>
          </div>
        </>
      )}
    </Card>
  );
}
