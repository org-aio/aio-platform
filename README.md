# Studio

Cargo 工件：`az-studio`

本插件解决约定式零代码应用的创建、编辑、校验、编译、发布和运行问题。它在同一个全栈 Dioxus crate 中拥有：

- 菜单树、页面、模型、函数图、路由和权限组成的 `ProgramDefinition`。
- Draft/Revision、强类型 Graph Patch、REST/SSE 和 Vibe Patch Agent。
- 页面内设置式画布、REST 功能定义、动态模型和浏览器 renderer。
- 类型/Effect/权限门禁、`ApplicationImage` 编译缓存、ArcSwap 发布和 Graph VM。

场景只是 `ProgramDefinition.menus` 的根节点，顶部工具条展示这些根菜单；不存在 `ContextDefinition` 或单独的场景表。Studio 使用一张树表管理菜单名称、图标、顺序、页面、路由、启用状态以及详情、编辑、删除权限，禁用节点不会进入发布后的 `ApplicationImage`。

`PageDefinition` 不保存组件树，只选择三种渲染声明：

- `ConventionFile`：按 `应用标识/页面标识` 匹配 Rudi `ConventionPageProvider`。期望文件固定生成到 `app/src/pages/{application}__{page}.rs`，补完文件并重新构建后生效。
- `TreeTable`：声明树模型、树标题/父级字段、表模型、关联字段、查询字段和显示列，运行时渲染左树右表。
- `CrudTable`：声明表模型、查询字段、显示列和分页大小，运行时渲染增删改查表格。

页面画布中央直接预览最终页面，右上角设置按钮打开元数据配置；不暴露组件目录、组件路径、属性面板或自由拖拽组件。

`PageDefinition.endpoints` 保存页面作为前端消费者声明的自定义 REST 接口。`CrudTable` 与 `TreeTable` 的查询、新增、修改、删除、导入、导出接口由编译器从布局和模型推导，不重复持久化；每条编译后接口都带 Rudi Provider 路由指令。自定义接口可以由 Vibe Agent 根据中文需求生成方法、相对路径、Path/Query/Header/Body 入参和响应 `data` 字段，运行时由 Rudi `RestFormPageProvider` 渲染并发起请求。

本插件不实现资产、IoT、SSH 等领域能力；这些插件只保留类型化 API 或 Capability，业务页面统一由 Studio 配置。正式程序只在 PostgreSQL 中保存，不持久化 Dioxus `Element`、SQL、HTML、CSS 或 JavaScript。约定文件是显式代码扩展点，不是页面配置真源。

Studio 的 Button、Badge、Card、Workflow 交互和设计令牌采用 [rust-ui/dioxus-ui](https://github.com/rust-ui/dioxus-ui) `91e8974` 版本。该项目是 shadcn 风格的源码 registry，因此这里只纳入 Studio 实际使用的控件，不维护业务组件目录。`app/assets/dioxus-ui.css` 是同一版本的样式产物。

```bash
cargo test -p az-studio
```
