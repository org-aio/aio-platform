# Studio

Cargo 工件：`az-studio`

本插件解决零代码应用的创建、编辑、校验、编译、发布和运行问题。它在同一个全栈 Dioxus crate 中拥有：

- 菜单树、页面、模型、函数图、路由和权限组成的 `ProgramDefinition`。
- Draft/Revision、强类型 Graph Patch、REST/SSE 和 Vibe Patch Agent。
- Rudi 动态组件目录、页面画布、属性绑定、逻辑图和浏览器 renderer。
- 类型/Effect/权限门禁、`ApplicationImage` 编译缓存、ArcSwap 发布和 Graph VM。

场景只是 `ProgramDefinition.menus` 的根节点，顶部工具条展示这些根菜单；不存在 `ContextDefinition` 或单独的场景表。Studio 使用一张树表管理菜单名称、图标、顺序、权限、页面、路由和启用状态，禁用节点不会进入发布后的 `ApplicationImage`。组件使用稳定 ID，例如 `ui.section`、`ui.button`，Rust 模块移动不会改变数据库身份。

本插件不实现资产、IoT、SSH 等领域能力；这些能力由对应插件以类型化 API 或 Capability 提供。正式程序只在 PostgreSQL 中保存，不持久化 Dioxus `Element`、源码、SQL、HTML、CSS 或 JavaScript。

Studio 的 Button、Badge、Card、Workflow 交互和设计令牌采用 [rust-ui/dioxus-ui](https://github.com/rust-ui/dioxus-ui) `91e8974` 版本。该项目是 shadcn 风格的源码 registry，因此这里只纳入 Studio 实际使用的组件，不链接包含 `fullstack` feature 和数百个无关组件的完整 registry crate。`app/assets/dioxus-ui.css` 是同一版本的样式产物。

```bash
cargo test -p az-studio
```
