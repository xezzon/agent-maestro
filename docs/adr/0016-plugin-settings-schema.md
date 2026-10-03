# 0016: 插件表单由 manifest 内联 settings_schema 驱动

有些插件需要用户选择工具专有的配置项（例如 Claude Code 要选定单一 model），这些项不属于 Maestro 的通用模型。我们决定给 manifest 加一个**可选**字段 `settings_schema`，内联一份 JSON Schema（draft 2020-12）描述该插件 `form`（ADR 0017）的形状；前端据此渲染表单。`settings_schema` 缺省即该插件没有需要用户填写的配置项。

选择**内联**而非「用字段指向一个外部 schema 文件」：schema 是纯元数据、体量小，与 manifest 同源、生命周期一致，落位目录里已有 `manifest.json`，无需多一个回源地址、多取一个资产、多一条失败路径与大小上限——那套机制本是给必须分开获取的 wasm（`entry`）准备的，搬一个几百字节的文件不值。前端用 `@rjsf/antd`（react-jsonschema-form 的 antd 主题）渲染，不自写表单生成器。宿主**不校验** `form`：宿主只搬运不解释，输入期由前端按 schema 校验，运行期由插件自行判断——把宿主拽进 schema 语义需要再背一个 JSON Schema 依赖与一条失败路径，收益只是「手改坏的文件更早报错」。

曾考虑并否决：manifest 指向外部 schema（https URL 或插件目录内相对路径，与 `entry` 回源同构）；宿主侧用 JSON Schema 校验 `form`。
