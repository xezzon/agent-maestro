# 0017: 插件表单由 manifest 内联 settings_schema 驱动

有些插件需要用户选择工具专有的配置项（例如 Claude Code 要选定单一 model），这些项不属于 Maestro 的通用模型。我们决定给 manifest 加一个**可选**字段 `settings_schema`，内联一份 JSON Schema（draft 2020-12）描述该插件 `form`（ADR 0018）的形状；前端据此渲染表单。`settings_schema` 缺省即该插件没有需要用户填写的配置项。

选择**内联**而非「用字段指向一个外部 schema 文件」：schema 是纯元数据、体量小，与 manifest 同源、生命周期一致，落位目录里已有 `manifest.json`，无需多一个回源地址、多取一个资产、多一条失败路径与大小上限——那套机制本是给必须分开获取的 wasm（`entry`）准备的，搬一个几百字节的文件不值。前端用 `@rjsf/antd`（react-jsonschema-form 的 antd 主题）渲染，不自写表单生成器。宿主**不校验** `form`：宿主只搬运不解释，输入期由前端按 schema 校验，运行期由插件自行判断——把宿主拽进 schema 语义需要再背一个 JSON Schema 依赖与一条失败路径，收益只是「手改坏的文件更早报错」。

**后果**：schema 的方言由 manifest 自己声明，前端校验器必须认得这个方言——声明 draft 2020-12 时 `@rjsf/validator-ajv8` 默认的 Ajv 只带 draft-07 的 meta-schema，编译即报 `no schema with key or ref "…/draft/2020-12/schema"`（表现为保存时校验失败），故用 `ajv/dist/2020` 的 `Ajv2020` 建校验器。`properties` 的顺序就是表单字段的渲染顺序，宿主透传时不得重排：`serde_json` 开着 `preserve_order`，否则默认的 `BTreeMap` 会按字典序重排字段。

曾考虑并否决：manifest 指向外部 schema（https URL 或插件目录内相对路径，与 `entry` 回源同构）；宿主侧用 JSON Schema 校验 `form`。
