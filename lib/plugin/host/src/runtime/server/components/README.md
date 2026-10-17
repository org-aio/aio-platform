# Component 安装与发布

原生 `aio:plugin@2.0.0` 整包进入独立的持久执行槽。发布校验只更新市场版本，租户安装由用户显式选择；父子依赖按租户校验，不自动安装子插件。页面转换仅用于共享壳导航，业务请求直接使用 v2 二进制契约。

`AIO_COMPONENT_DATABASE_URL` 指向具有创建隔离角色权限的专用 PostgreSQL，数据库必须撤销 PUBLIC 权限。`AIO_COMPONENT_HOME` 是发布目录之外的持久目录，保存 0600 的 `keyring.json` 和 `objects/`；未配置数据库时不开启原生 Component 安装。

安装记录是版本激活的提交点。安装失败恢复上一执行槽，重启时按安装记录恢复执行槽；停用和卸载保留业务 schema、对象及版本历史。声明的业务权限由当前已启用安装版本派生，租户全部成员（包括新加入成员）自动获得，无需配置角色。停用、卸载或版本回退后，下一次请求即按当前版本重新授权。

派生权限使用 `component:<source-id>:<permission>`，组件内仍调用原始权限名，宿主按当前执行来源映射。不同插件声明同名权限不会互相授权，也不能覆盖宿主权限。

系统包另用 `plugin.required_permissions = ["dictionary:manage"]` 声明访问门槛。`access.rs` 按安装摘要读取由不可变清单派生的索引，再通过同一 Dill 容器中的身份和授权 Provider 查询当前会话、成员与角色；该字段不参与 `permissions()` 的成员授权。目录隐藏受保护页面及账户、设置入口，保留安装管理条目。挂载、资源和续期复核同一门槛，`dispatch.rs` 在任何执行器之前要求仍有效的交互上下文；跨插件派生链保留原始用户，后台及设备请求不能借用管理权限。开发装配直接读取当前清单，和持久安装保持一致。

前端挂载只返回当前已验证整包的静态资源摘要和大小，不返回后端或迁移文件。入口通过 HTML 解析器注入宿主资源桥和固定模块加载器，让 Compose 的模块、Wasm 与包内 fetch 复用会话缓存；HTML 与模块加载器继续经过实时授权，资源桥不开放额外网络能力。后台预热由宿主目录统一编排。

## 开发装配

本地控制面使用内容摘要验证的源码快照和内存安装绑定，数据库、密钥和对象存储仍复用正式能力实现。
Component 前端单独变化时保留同一执行实例；本地服务由 CLI 启动，宿主准备私有配置、Unix 通信和依赖授权。
沙箱重启重新装载当前运行集合，不向市场版本表写入开发包或伪造提交。

## 权限与菜单验证

`tenant_installation_grants_members_and_menu_visibility_preserves_runtime` 在独立 `component_market_test` PostgreSQL 与真实 WIT 测试组件上验证成员自动授权、跨租户与已移除成员拒绝、旧权限索引回填、升级回退、启停卸载，以及隐藏菜单不改变运行实例和授权。设置 `AIO_COMPONENT_TEST_DATABASE_URL` 与 `AIO_TEST_HEALTHY_COMPONENT` 后运行 `cargo test -p az-plugin-host --features wasm,sidecar,rhai,typescript --lib components:: -- --include-ignored`。

`tests/component_execution/access.rs` 用真实 HTTP 和替代执行器覆盖宿主权限隔离、旧会话快照撤权、资源及续期拒绝、身份头伪造、跨租户、提供者查询失败、升级回滚、重启回填和默认拒绝。运行 `cargo test -p az-plugin-host --no-default-features --features server --test component_executors access:: -- --ignored`。`protected_wasm_and_sidecar_require_live_interaction_through_the_broker` 使用真实 Wasm、开发态 Sidecar 和 Unix socket broker，覆盖受保护目标跨执行器替换、后台拒绝、派生上下文归属与失效、菜单和设置权限；需要 `wasm,sidecar` feature 和上述两个测试环境变量。这些用例不依赖浏览器或真实容器监督器，不能作为容器隔离或产品系统包迁移完成的证据。

`installation` 统一安装事务、启停卸载与回滚；`delivery` 在同一变更锁内再次检查启用状态、市场版本及排除摘要，自动更新既有安装。回滚仅排除当前市场版本，不永久固定插件；失败沿用安装路径的旧实例恢复。

`bootstrap` 供可选 `ComponentBootstrapPlugin` 执行部署首次安装。先整批校验固定摘要及父子关系，再复用发布和安装；与安装同事务写入的 `component_initializations` 保留租户选择，卸载后仍存在。引导不会覆盖最新发布，也不会重装已管理的来源。参见 [整包首次引导](../../../../../../../docs/plugin/component-bootstrap.md)。

`management` 为部署方按完整包摘要信任的系统插件提供公共宿主管理能力。Wasm、Sidecar broker 和前端共用真实交互登记与实时权限检查；不允许跨插件转授。前端直接调用管理入口，可以管理自身生命周期；执行器中的同步生命周期变更被拒绝。撤信任保留安装记录并暂停执行，恢复后沿用原摘要和代数。参见 [系统整包管理](../../../../../../../docs/plugin/component-management.md)。

## 可替换执行器

从宿主 `plugins/component_execution.rs` 进入。`WasmComponentPlugin` 与 `SidecarPlugin` 提供默认实现，`ComponentExecutionPlugin` 汇集 Dill 中的 `ComponentExecutor`，`ComponentResourcesPlugin` 在资源就绪后初始化实现，`ComponentsPlugin` 按持久化安装记录恢复实例。

`execution.rs` 中的 `ComponentExecutor` 相当于 Kotlin 的接口。应用插件可以把自定义实现绑定到这个接口，并省略对应默认插件：

```rust,ignore
assembly.services
    .add_value(MyExecutor::default())
    .bind::<dyn ComponentExecutor, MyExecutor>();
```

具体实现的 `TypeId` 只用于当前运行时去重和判断是否切换实现；不能写入配置或数据库。`supports` 根据已验证整包选择实现，必须是无副作用且稳定的判断。缺失和歧义匹配都会失败；展示用的 `description.process` 不决定执行路径。

请求先检查当前租户的安装记录、摘要和页面授权，再经 `dispatch.rs` 找到绑定的执行器。原生实现还要核对自己的实例摘要，避免升级期间用新实例处理旧请求。`ExecutionContext` 使用弱引用连接宿主，按整包授权准备租户资源，并从宿主同一个 Dill 容器取得服务。执行器不得长期强持有该容器或宿主，以免形成循环引用。

`installation.rs` 统一版本提交与补偿：

1. 选中候选执行器，记住已启用的旧整包与执行器。
2. 激活候选，提交安装事务。
3. 失败时清理异类候选，再按旧安装记录恢复；清理失败仍继续恢复。
4. 提交成功后更新路由绑定，再释放不同类型的旧执行器资源。

`activate` 与 `restore` 都接收具体整包；后者必须修复未完成提交或被停用的实例，不能只相信本地缓存。默认 Wasm 实现维护持久槽、引擎缓存和开发槽；默认 Sidecar 实现复用监督器、Unix socket broker、数据库转发及其资源所有权。

初始化失败与 panic 会在资源交出前补偿；关闭按反序尝试所有实现并汇总错误。`shutdown` 必须允许部分初始化且只释放当前宿主拥有的资源，不能删除安装记录和持久版本。开发 Sidecar 由 CLI 启停，宿主只释放自身 broker 与转发。默认 Wasm 的前端刷新继续复用后端实例。

这是可信原生扩展接口，具体实现负责自己的执行配额、进程隔离和实例取消。脚本自举使用独立的增强服务，其权限和版本提交仍由宿主控制。v2 整包执行器与旧 v1 的 `WasmExecutor` / `ProcessExecutor` 分别装配，组件资源也通过独立 Provider 替换。

## 可替换资源

从 `plugins/component_resources.rs` 注册 `ComponentDatabaseProvider` 与 `ObjectStorageProvider`。默认组合选择 `PostgresComponentPlugin` 和 `FileObjectStoragePlugin`；定制组合可以省略任一默认实现，在自己的插件中绑定同一接口。`ComponentResourcesPlugin` 会在资源创建前拒绝缺失或多个绑定，不回退默认实现。

Provider 的 `open` 返回本次宿主拥有的后端。资源准备、执行器初始化、实例恢复沿用同一生命周期；停止时先退出执行器，再关闭对象后端和数据库。对象后端初始化失败或 panic 会补偿已打开的数据库，关闭某个后端失败仍会继续清理其他后端。Provider 在 `open` 内失败时负责尚未返回资源的清理。

`resources.rs` 按包授权创建 `InvocationResources`，校验来源和租户后交给 Wasm 或外部执行器。Sidecar 数据库配置也通过此入口；Sidecar 当前仍未开放对象存储能力。对象读写的租户命名空间、键校验和大小限制由运行时句柄执行，后端只消费已校验地址。

`tests/component_execution/resources.rs` 移除默认 Provider，使用预先分配的 PostgreSQL 连接和测试对象后端，验证两个外部执行器共享资源、Wasm 的真实 WIT 读写与撤权、Sidecar 数据角色和表权限、持久槽恢复、错误归属拒绝及失败补偿。执行运行时另外以真实 SQLite 替代 `DatabaseBackend` 与 `ComponentReleases`，覆盖 WIT 读写、并发发布、失败回滚和持久恢复，见 runtime 的 `tests/database_backends.rs`。正式产品仍默认使用 PostgreSQL；平台安装目录尚未抽离，SQL 方言与迁移也不会自动转换。

数据库后端返回中立事务与值模型，发布槽通过 `ComponentDatabase::releases()` 获取持久存储。Sidecar 只消费同一绑定导出的 `ProcessDatabaseConnection`，按声明的 TCP/Unix 上游建立私有代理，不再解析 PostgreSQL 驱动选项。不支持网络直连的后端明确拒绝 Sidecar；不允许回退到平台数据库。
