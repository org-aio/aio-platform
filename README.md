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

模型字段的“查询”和“排序”是页面能力开关，不再与索引用途重复。索引只描述字段集合和是否唯一，联合唯一约束由多字段索引表达。字段关联保存两端模型字段和 `OneToOne`、`ManyToOne`、`OneToMany`、`ManyToMany` 基数；Studio 设置一端时同时写入对端，编译器要求两端基数互逆且类型相符。因此 `Department.users` 与 `User.departments` 使用两条互指的 `List<Object>` 关系，而不是字符串表达式。

模型还可以声明命名查询和模型级校验。查询条件使用本模型字段或关联模型字段的参数化条件，可表达“部门名称包含参数且关联用户名称包含参数”；校验覆盖单字段长度/数值/正则、列表最小/最大项数与元素去重，以及联合必填、至少一个必填和条件必填。所有这些定义都进入 `ProgramDefinition`，由编译器做字段归属、关系完整性和范围校验。

设备能力的术语参考 W3C WoT：字段可作为 `Property` 的数据模式，命令和异步通知分别应落在 `Action` 与 `Event`，但 Studio 不把内部的数据库关联和查询表达式伪装成完整 Thing Description。模型可组合绑定租户、创建/更新人、创建/更新时间、逻辑删除、删除人/时间与版本号等审计角色；每个角色都绑定稳定字段 ID，勾选缺失角色时会生成默认字段，取消角色只移除审计语义而不删除已有业务数据。

`PageDefinition.endpoints` 保存页面作为前端消费者声明的自定义 REST 接口。REST 方法与相对路径就是接口身份，不再重复保存接口标识；显示名称可省略，空值时从路径末段推导。`CrudTable` 与 `TreeTable` 的查询、新增、修改、删除、导入、导出接口由编译器从布局和模型推导，不重复持久化；每条编译后接口只保留实际使用的 Rudi Provider 路由指令。自定义接口可以由 Vibe Agent 根据一次性的中文需求生成方法、相对路径、Path/Query/Header/Body 入参和响应 `data` 字段，需求文本不进入接口元数据，运行时由 Rudi `RestFormPageProvider` 渲染并发起请求。

本插件不实现资产、IoT、SSH 等领域能力；这些插件只保留类型化 API 或 Capability，业务页面统一由 Studio 配置。正式程序只在 PostgreSQL 中保存，不持久化 Dioxus `Element`、SQL、HTML、CSS 或 JavaScript。约定文件是显式代码扩展点，不是页面配置真源。

Studio 的交互控件采用 [Dioxus Components](https://github.com/DioxusLabs/dioxus-components) `bf007c15` 的源码 registry，通过官方 `dx components add` 纳入实际使用的 Button、Badge、Input、Textarea 和 Checkbox。`app/assets/dx-components-theme.css` 提供同一提交的设计令牌，`app/assets/tailwind.css` 只提供整站布局工具类；上游未提供 Table，功能定义表格直接使用语义化 HTML，不维护第二套本地设计系统。完整网页开发约定见仓库 Skill `.agents/skills/aio-dioxus-web`。

```bash
cargo test -p az-studio
```
