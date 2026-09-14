# aio-platform 插件协议

aio-platform 提供开发契约和 CLI，aio-idea 宿主负责租户组合、权限、安装事务和回滚。一个功能仓库同时拥有页面、服务和共享逻辑，子插件可以与父插件同库。Git 安装锁定源码提交；插件中心发布锁定 `.aio-plugin` 包内容 SHA-256，完整二进制与元数据存入 PostgreSQL，不以市场条目作为 Rust 运行时身份。

## 运行形态

| 形态 | 适用能力 | 生命周期 |
| --- | --- | --- |
| `rust-source` | 与宿主同版本的 Dioxus 页面、Dill Service/Controller | 拉取源码后隔离编译，整套版本原子切换 |
| `page-definition` | 多语言生成的静态页面定义 | 校验 JSON 后直接按租户挂载，可单独卸载 |
| `wasm-component` | 多语言纯计算、PageDefinition 生成、受限请求处理 | Wasmtime 实例化，按声明授予 WASI 能力，可单独卸载 |
| `process` | JVM/Node、数据库驱动、长任务、第三方 SDK | 独立进程或容器，健康检查后挂载路由，停止即卸载 |

Wasm 是多语言 ABI 的优先选项，但不是所有插件的唯一运行时。浏览器插件不得直接接管宿主 DOM；它返回 `PageDefinition` 和事件结果，由宿主统一渲染。服务端 Wasm 只获得清单声明且租户允许的 WASI 能力。需要任意网络、原生数据库驱动或长期后台任务时使用 `process`。

这里的运行形态描述多语言 artifact 如何被宿主执行，不是 Rust trait 的注入身份。Rust 插件固定使用源码装配，不接受 `--runtime` 选择；页面扩展实现 `ApplicationPlugin` 后由 Dill 聚合，Service 和 Controller 按具体类型注册和构造，运行时唯一性统一由 `TypeId` 判断。Kotlin 默认使用 `process`，TypeScript 默认使用 `wasm-component`，只有静态页面或非默认目标需要显式覆盖 `--runtime`。

## 仓库清单

Rust 源码模式在同一个 Git 仓库内使用 client、server crate，可增加 shared crate 保存共享模型和逻辑；不得拆成前后端两个仓库：

```toml
[plugin.client]
path = "client"

[plugin.server]
path = "server"
```

至少声明一端。`client` 导出 `register(&mut dill::CatalogBuilder)`；`server` 导出 `register(&mut dill::CatalogBuilder) -> anyhow::Result<()>` 和 `router(&dill::Catalog)`。宿主生成的装配代码会传播服务端注册错误，运行时扩展只依赖具体 Rust 类型的 `TypeId`。

在线插件统一声明已经构建好的运行产物，再通过 `aio plugin package` 打成可搬运的二进制包：

```toml
[plugin.runtime]
kind = "wasm-component"
artifact = "dist/plugin.wasm"

[plugin.capabilities]
network = []
filesystem = []
database = false

[[plugin.subplugins]]
id = "hello-screen"
pages = ["hello"]
routes = ["hello"]
account_actions = ["hello"]
```

`process` 仓库必须把启动和停止契约明确写入清单，不允许安装器猜测脚本：

```toml
[plugin.runtime]
kind = "process"
artifact = "dist/plugin.jar"
host_version = ">=2026.5.10"
container_image = "eclipse-temurin:21-jre@sha256:5c67d24ee8e3dd810b2a0cb6c3827ced2ac5d22729538f90b36c2b9d77678bb8"
entrypoint = ["java", "-jar", "{artifact}"]
health_check = "/health"
shutdown_timeout_seconds = 10
```

`container_image` 必须锁定 digest，宿主仅使用预置镜像且不在安装期拉取。`{artifact}` 由监督器替换为容器内只读产物路径。进程在固定 `8080` 端口提供 `health_check`、`GET /aio/definition` 和清单声明的业务路由；端口不映射到公网，只由宿主代理。

运行时子插件可通过 `account_actions` 把已有页面加入账户区。动作值必须等于同一仓库的已声明页面 ID；壳从该页面推导标题、图标和权限，动作 ID 会按 Git 来源命名空间化。运行时插件不能用账户动作声明退出、修改权限或其他任意宿主命令。

只贡献静态页面定义的 Kotlin/TypeScript 插件可以声明 `kind = "page-definition"`，artifact 是 `PageDefinition` JSON 数组。它适合由跨平台 common 模型生成页面；需要动态后端请求处理时再升级为 Component。

Wasm Component 必须实现 [`aio:plugin/page@1`](wit/page.wit)：`definition() -> string` 返回 `PageDefinition` JSON 数组，`handle(request: string) -> string` 接收带明确 `kind` 的 `PluginRequest`，返回包含 `status`、`content_type` 和 `body` 的 `ComponentResponse` JSON。宿主为每个租户、来源和 revision 创建独立实例，实例无默认 WASI 权限并在每次调用前重置 fuel；停用、卸载、回滚和租户组合切换会销毁对应实例。

[`schema/`](schema/) 保存上述模型的 JSON Schema。执行 `aio plugin schema [<输出目录>]` 可生成相同契约，用于多语言类型生成、IDE 补全和 CI 结构校验；完整语义仍以 `aio plugin validate` 为准。

源码宿主 CLI 只负责 `rust-source` 的 Cargo 装配。公网运行时支持 `page-definition`、`wasm-component` 和 `process` 在线发布、安装、停用、启用、卸载和回滚。`process` 运行在按租户、来源和 revision 隔离的容器网络中，使用非 root 用户、只读根文件系统、资源配额与 capability 丢弃；当前只开放零额外能力档，非空网络、文件系统或数据库声明会在启动前被拒绝。

当前 `PageDefinition.body` 支持以下稳定形态：

```json
{
  "id": "hello",
  "label": "Hello",
  "icon": "panels-top-left",
  "scene": { "id": "workspace", "label": "工作区" },
  "required_permission": null,
  "body": { "kind": "counter", "title": "Hello Runtime", "button": "计数加一" }
}
```

顶部场景是菜单树的 root。页面用 `menu_path` 声明 root 下任意深度的目录路径；目录不拥有业务页面，多个独立插件可以向相同目录贡献叶子。例如 RBAC 与字典插件共同进入系统管理，文件插件进入基础设施：

```json
{
  "id": "dictionary",
  "label": "字典管理",
  "icon": "book-open",
  "scene": { "id": "system", "label": "系统" },
  "menu_path": [
    { "id": "system-management", "label": "系统管理", "icon": "settings" }
  ],
  "body": { "kind": "text", "title": "字典管理", "content": "..." }
}
```

同一目录 id 的场景、父目录、标题和图标必须一致；目录路径不能循环，目录 id 不能与页面 id 冲突。省略 `menu_path` 的页面是该场景 root 的直接子页面。账户区贡献的页面仍是独立全屏入口，不进入场景树。

### KMP 插件自定义顶部分组与多级菜单

`aio plugin init my-kmp-plugin --language kotlin --title "我的插件"` 生成的项目，在 `backend/service/resources/pages.json` 使用 `children` 树编写导航。根节点是顶部分组，目录节点带 `children`，叶子页面带 `body`。不需要为每个页面重复填写 `scene` 和 `menu_path`；公共解析层展开后统一校验、存储和合并菜单。

以下是完整双页面示例。两个菜单复用现有 `index.html`，可以直接验证导航；不同业务页面需要实现各自的前端入口：

```json
{
  "id": "my-business",
  "label": "我的业务",
  "children": [
    {
      "id": "business-operations",
      "label": "运营中心",
      "icon": "folder",
      "children": [
        {
          "id": "business-analysis",
          "label": "数据分析",
          "icon": "folder",
          "children": [
            {
              "id": "business-dashboard",
              "label": "数据大屏",
              "icon": "layout",
              "required_permission": null,
              "body": { "kind": "frontend", "entry": "index.html" }
            },
            {
              "id": "business-reports",
              "label": "报表管理",
              "icon": "layout",
              "required_permission": null,
              "body": { "kind": "frontend", "entry": "index.html" }
            }
          ]
        }
      ]
    }
  ]
}
```

导航效果：

```text
顶部：我的业务
侧栏：运营中心
      └─ 数据分析
         ├─ 数据大屏
         └─ 报表管理
```

- 一个顶部分组写成上面的根对象，多个分组写成根对象数组。页面直接放在根的 `children` 下时没有中间目录。
- 目录的 `children` 不能为空，目录最多八层。叶子必须有 `body`，不能同时包含 `children`；未知字段和拼写错误会拒绝。
- `required_permission` 只声明在叶子上，例如 `"reports.read"`，由宿主按页面检查权限。目录不接受权限字段，也不会隐式继承或覆盖叶子的权限。
- 页面 id 必须唯一。调整已发布页面的菜单位置时保留原页面 id 和 body；新增页面后必须同步清单中的 `plugin.subplugins[].pages`。
- 不同插件可以共同贡献相同目录，但目录 id 的场景、父节点、标题和图标必须一致。不同分支下的同名目录需要不同 id；目录 id 不能与页面 id 冲突。
- 菜单配置不会自动生成 Compose 页面。如果报表需要独立页面，先实现并构建对应 HTML 到 `dist/frontend`，再将 `body.entry` 改成该 HTML 的相对路径；不能只填不存在的文件名或查询串。

在生成的插件根目录执行 `sh scripts/build.sh`，然后执行 `aio plugin validate .`。构建后按项目发布流程发布新版本并激活，宿主才会采用新菜单。校验需要构建产物，尚未构建时提示 `dist/plugin.jar` 不存在是预期结果。初始化命令目前没有 `--scene` 或 `--menu-path` 参数，直接修改 JSON 即可。

上面的双页面示例还需将 `aio-plugin.toml` 现有 `[[plugin.subplugins]]` 中的 `pages` 改为 `['business-dashboard', 'business-reports']`。只改目录或标题且页面 id 不变时，无需改这个列表。

树状文档统一适用于静态 `page-definition` 产物、Wasm `definition` 和 process `/aio/definition`。需使用包含 `parse_page_definitions` 的 CLI 与宿主；此前发布的 npm `2026.5.10` 不含此功能。内部协议与数据库继续存储展开后的 `PageDefinition` 列表，不需要数据迁移。

生成项目中的说明来源见 [KMP 模板 README](../../cli/templates/plugin/fullstack/kotlin/README.md)。

纯文本页面把 `body` 改为 `{ "kind": "text", "title": "...", "content": "..." }`。`counter` 由宿主维护当前浏览器中的本地计数，适合静态示例，不调用插件运行时，也不在刷新后保留值。渲染阶段的 `UiOp`、HTML、CSS 和 Dioxus `Element` 都不能作为持久化协议。

需要由插件处理按钮事件时，`wasm-component` 或 `process` 页面使用 `actions` 页面体：

```json
{
  "kind": "actions",
  "title": "租户计数器",
  "content": "计数：0",
  "state": { "count": 0 },
  "actions": [{ "id": "increment", "label": "+1" }]
}
```

宿主在点击后发送 `{"kind":"page_action","page_id":"...","action_id":"...","tenant_id":"...","user_id":"...","body":<当前 PageBody>}`。Wasm Component 从 `handle` 的响应 `body` 返回 `{"body": <PageBody>}`；process 插件在 `POST /aio/action` 直接返回相同结果。租户、用户和当前页面体只由宿主注入，插件应根据 `body.state` 计算新状态，不依赖进程内可变状态。动作结果通过校验后按租户、Git revision 和页面写入 PostgreSQL，刷新与宿主重启后继续生效；不同 revision 各自保留状态，卸载时清除。结果只能替换当前页面体，页面身份、导航、权限、场景和动作声明仍以安装时校验并保存的 `PageDefinition` 为准。状态最多 64 个字段且序列化后不超过 64 KiB。`page-definition` 没有运行实例，因此不能声明 `actions` 页面体。

## 租户组合

在线租户组合保存在 PostgreSQL，锁定已发布包的完整内容 SHA-256。源码装配和首次引导组合仍可跟踪 Git 分支、标签或提交，生产锁文件保存解析后的完整 Git SHA：

```toml
[[plugins]]
git = "https://github.com/example/aio-plugin-orders.git"
rev = "9d0b7d16f9f5a4c5a3b4c0e1e6c43ae8d47aa001"
```

安装流程为：从插件中心读取二进制包、校验内容摘要与能力声明、恢复版本缓存、实例化或健康检查、原子切换租户活动版本。未收录且未发布的 Git 仓库走受限拉取流程，仍须有已构建产物。公网安装器不执行 `npm`、`pnpm`、Gradle、Cargo 或仓库自定义脚本；构建可以在作者本机完成，不依赖 CI，也不要求向 Git 提交二进制。监督器停止失败时不修改活动组合，数据库切换失败时恢复先前实例；卸载同步停止进程/Wasm 实例。

## 市场

正式插件市场读取 PostgreSQL。`aio plugin publish` 上传的包通过验证与激活后，清单元数据直接进入官方市场。`marketplace/registry/` 是可选的冷启动发现目录，不是在线市场的唯一来源；租户也可以配置未收录 Git 仓库。发布凭证与租户安装权限分开管理。

语言细节见 [Rust 规约](rs-plugin-convention.md)、[Kotlin 规约](kt-plugin-convention.md)、[TypeScript 规约](ts-plugin-convention.md) 和 [在线发布规约](publish.md)。

CLI 可以直接生成七种仓库骨架。前三条是各语言的常规初始化，不需要指定运行目标：

```bash
aio plugin init aio-plugin-rust --language rust
aio plugin init aio-plugin-kmp --language kotlin
aio plugin init aio-plugin-ts --language typescript

# Kotlin/TypeScript 的静态页面或非默认目标
aio plugin init aio-plugin-kmp-pages --language kotlin --runtime page-definition
aio plugin init aio-plugin-kmp-component --language kotlin --runtime wasm-component
aio plugin init aio-plugin-ts-pages --language typescript --runtime page-definition
aio plugin init aio-plugin-node --language typescript --runtime process
```

省略 `--language` 时使用 Rust。Rust 不暴露运行目标选项；Kotlin/TypeScript 省略 `--runtime` 时分别选择 `process` 和 `wasm-component`。尚未发布的语言/运行目标组合会在创建目录前失败，不生成半成品仓库。

提交市场前必须在插件仓库执行语言自身的构建与测试，然后使用宿主共享校验器：

```bash
aio plugin validate
aio plugin package . --version 1.0.0
aio plugin publish dist/plugin.aio-plugin
```

`validate` 不执行仓库脚本；它只读取已生成 artifact，校验清单、页面声明与 Wasm Component WIT 边界。对 `wasm-component`，校验器还会在无导入、限 fuel/内存的 Wasmtime 实例中调用 `definition()`，验证实际返回的 `PageDefinition` 与子插件页面声明一致。`package` 校验预构建产物并生成完整二进制包，`publish` 直接上传并等待健康检查和原子激活；两者都不要求已提交产物或 GitHub Actions 环境。
