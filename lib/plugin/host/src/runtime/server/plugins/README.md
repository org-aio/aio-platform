# 宿主能力插件

宿主通过 `Plugin<RuntimeAssembly>` 复用 `az-plugin-core` 的 TypeId 身份和依赖排序，通过 Dill 提供服务。`RuntimeState` 不再拥有剪贴板、个人配置、设备、设备视图、数据库、包仓库和执行资源的固定字段；HTTP、组件 broker 和 sidecar 使用同一个服务容器。

先看 `RuntimeAssembly`，再看 `builtins.rs` 中的默认平台组合。插件可以注册服务、贡献 HTTP 路由，以及注册恢复、就绪和停止动作。所有恢复动作成功后才执行就绪动作；省略交付插件也会省略它的后台启动动作。`require_service::<dyn Trait>()` 检查接口的唯一绑定，允许应用提供其他实现。`Plugin::dependencies()` 只声明必须先构建的具体插件类型，不能用它强制绑定某个默认服务实现。

默认宿主使用 `RuntimeState::initialize(config, identity)`。定制宿主从自己的 Dill 容器解析插件组合，交给 `initialize_with_plugins`：

内置接口位于 `generated::{worker, worker_webview, clipboard, personal_config}`。例如保留 `ClipboardPlugin` 的认证和路由，省略 `ClipboardServicePlugin`，再绑定自定义 `dyn ClipboardService`，即可更换实现。服务返回的 HTTP 错误使用公开的 `runtime::server::http_error::RuntimeError`。同一接口出现多个实现会使装配失败，不采用覆盖或隐式回退。

```rust,ignore
let catalog = dill::Catalog::builder()
    .add::<MyServicePlugin>()
    .add::<MyRoutesPlugin>()
    .build();
let plugins = az_plugin_core::PluginGroupBuilder::default()
    .add::<MyRoutesPlugin>()
    .add::<MyServicePlugin>()
    .resolve(&catalog)?;
let state = RuntimeState::initialize_with_plugins(config, identity, plugins).await?;
```

插件的 `build` 负责声明装配，不应启动后台任务或执行迁移。将资源初始化与迁移交给 `on_initialize::<Self>`，将异步恢复交给 `on_start::<Self>`，将后台工作交给 `on_ready::<Self>`，将资源清理交给 `on_stop::<Self>`。路由通过 `state.service::<dyn MyService>()?` 获取服务。长连接在升级前解析服务，并把同一个 `Arc` 传入会话；身份与撤权检查仍在会话期间持续执行。

生命周期顺序来自内核的 `App::plugin_order()`，身份始终是具体类型的 `TypeId`。进入插件的初始化动作前登记停止动作，因此停止回调必须能处理部分初始化，也不能删除安装记录或已经提交的业务数据。所有启动动作成功后才执行就绪动作；失败、panic 或取消会先停止接收新任务、等待已有工作完成，再按依赖反序执行停止回调。一个回调失败不会跳过其他插件，错误会汇总返回；重复关闭不重复执行回调，并保留第一次的结果。

```rust,ignore
assembly.on_ready::<Self>(|state, tasks| Box::pin(async move {
    let cancelled = tasks.cancellation();
    tasks.spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = cancelled.cancelled() => return Ok(()),
                job = next_job(&state) => process_job(&state, job?).await?,
            }
        }
    })
}));
assembly.on_stop::<Self>(|state, _tasks| Box::pin(async move {
    state.service::<MyResources>()?.close().await
}));
```

请求入口通过 `state.plugin_tasks::<MyPlugin>()?` 使用同一个任务作用域。不能用脱离作用域的 `tokio::spawn` 保存无限后台循环。收到取消后，有限持久化工作应先完成提交或补偿，再返回；交付循环在轮次边界退出，未领取的发布任务仍留在队列中。停止不会强行中断正在提交的发布。任务结束或 panic 的错误在任务回收与关闭时报告，保留有限错误样本避免内存增长。

应用持有 `RuntimeState`，在退出时显式调用 `state.shutdown().await`，同时关闭监听器。初始化等待被取消后会协作停止并补偿；关闭等待被取消后清理任务仍会继续。运行时路由关闭后返回 503，已进入的处理先完成，长响应流在下一次轮询时结束；gRPC trailers 原样转发。独立升级的 WebSocket 会话不属于 HTTP 响应体，需要其所属服务自行管理。直接调用 Dill 服务的原生代码也必须停止派发工作。不要从正在被等待的请求、任务或生命周期回调内部等待 `shutdown()`。

停止只释放当前宿主拥有的 Sidecar、broker、数据库转发与 Wasm 内存实例，保留 PostgreSQL 中的安装状态、授权与活动版本。旧监督器报告为已存在的进程不会被当成本次创建的资源停止。开发 Sidecar 的进程由 CLI 所有，宿主只关闭自己的 broker 和转发。协作取消要求回调和任务最终返回；无限阻塞或忽略取消的原生插件仍需要外部监督器处理。关闭有错误时仍执行其余清理，下一次启动由现有恢复/孤立实例清理流程继续恢复。

这是原生宿主插件的装配边界，还不包含在线卸载、单插件重启或脚本服务代理。HTTP 路由不能替代能力授权。数据库连接、密钥来源与执行器创建已由所选插件管理；组件内部具体执行器、存储及分发契约仍待迁移，完整目标见 [一切皆插件验收清单](../../../../../../../docs/plugin/everything-as-plugin.md)。

验证：`cargo test -p az-plugin-host --features server plugins::tests`。真实 HTTP 精简组合验收使用无效数据库地址，无需数据库即可运行。

故障验收：`cargo test -p az-plugin-host --features server lifecycle_tests` 覆盖真实端口释放、启动/就绪失败、取消等待、回调 panic、后台工作提交与 HTTP 收尾。`process::shutdown_tests` 使用真实 Unix socket 监督器协议验证只停止本次创建的进程；Component 的持久化停止验收复用 `publication_preserves_running_singleton_process`。

空组合或只有自定义路由的组合不连接 PostgreSQL、不读取密钥、不创建 Wasm 或 Sidecar 资源。配置值只由实际安装的插件消费。

## 初始化与停止顺序

1. 检查插件重复、依赖缺失和循环，构建服务声明与路由，检查服务绑定唯一性；此时不解析需要异步资源的服务。
2. 按插件依赖顺序执行 `on_initialize`；开发宿主也执行这一阶段。
3. 解析 `require_service` 声明的服务；未提供的资源、构造失败或 panic 都触发反序补偿。
4. 正常宿主执行 `on_start` 恢复状态，再执行 `on_ready` 开启后台任务。开发宿主跳过这两个阶段。
5. `shutdown` 关闭请求准入，等待插件任务，然后反序执行 `on_stop`。停止钩子必须允许部分初始化，且不能删除持久化安装和活动版本。

应用必须在退出时调用 `state.shutdown().await`。初始化或停止的等待方被取消，不会中断独立清理任务。停止是协作式的；无响应的原生代码需要监督器处理。此机制尚不支持原生插件在线卸载或单插件重启。

## 声明异步资源

在插件的 `build` 中声明资源与钩子，类型依赖仍写在 `Plugin::dependencies` 中：

```rust,ignore
assembly.declare_resource::<Connection>()?;
assembly.on_initialize::<Self>(|state, _tasks| Box::pin(async move {
    let provider = state.service::<dyn ConnectionProvider>()?;
    let connection = provider.connect().await?;
    state.service::<Resource<Connection>>()?.provide(Arc::new(connection))
}));
assembly.on_stop::<Self>(|state, _tasks| Box::pin(async move {
    if let Some(connection) = state.service::<Resource<Connection>>()?.initialized() {
        connection.close().await?;
    }
    Ok(())
}));
```

业务服务直接注入 `Connection` 或 `Arc<Connection>`；`Resource<T>` 是 Dill 中的一次初始化槽，负责将异步资源交给正常的同步 DI，不是第二套注册表。重复声明和重复提供都失败。依赖该资源的初始化钩子必须声明对资源所有者插件的 TypeId 依赖。资源中需要保存自身所属容器时只保存 `CatalogWeakRef`，避免容器与单例相互持有。

## 默认基础设施与替换方式

| 插件 | 所有权与依赖 |
| --- | --- |
| `DatabasePlugin` | 从唯一 `DatabaseProvider` 取得宿主拥有的 `PgPool`，停止时关闭；默认 `PostgresPlugin` 提供连接实现 |
| `KeyringPlugin` | 从唯一 `KeyringProvider` 加载共享密钥；默认 `FileKeyringPlugin` 读取文件 |
| `StorePlugin` | 依赖数据库，迁移平台表及清理开发会话的旧绑定 |
| `RepositoryPlugin` | 提供包缓存与仓库能力 |
| `ProcessPlugin` / `WasmPlugin` | 持有旧进程 / Wasm 执行资源，并负责停止 |
| `RhaiPlugin` / `TypeScriptPlugin` | 向 Dill 提供对应的 `ScriptExecutor`，省略后不接受该语言 |
| `ScriptRuntimePlugin` | 汇集所选脚本执行器，统一候选与样例验证，不连接数据库或创建文件 |
| `EnhancementServicePlugin` | 依赖数据库与脚本运行时，提供统一的增强版本、授权和调用服务 |
| `WasmComponentPlugin` / `SidecarPlugin` | 向 Dill 提供 v2 整包的 `ComponentExecutor`；省略后不初始化对应引擎或监督器客户端 |
| `ComponentExecutionPlugin` | 汇集整包执行器并拒绝重复类型；本身不创建执行资源 |
| `ComponentResourcesPlugin` | 依赖存储、密钥、增强服务与整包执行器，配置组件存储时初始化所选实现，停止时卸载实例并关闭组件连接 |

替换数据库连接方式时保留 `DatabasePlugin`，省略 `PostgresPlugin`，在自己的插件中使用 Dill 绑定 `dyn DatabaseProvider`；密钥同理。Provider 返回的连接池属于本次宿主，会在关闭或初始化失败时关闭，不应返回另一个宿主仍在使用的池。所有服务和 Sidecar 消费同一个容器提供的实例。

该替换点允许外部连接管理与密钥来源；平台的数据模型仍使用 PostgreSQL，组件数据库 provisioner 与对象存储尚未抽象。旧 `WasmPlugin` / `ProcessPlugin` 管理 v1 资源，与 v2 整包执行器插件分开。

## 脚本执行器

读取 `enhancement_execution.rs` 的装配，再看 `az-plugin-enhancement` 的 `ScriptExecutor` / `ScriptProgram` 契约。应用的插件向同一个 `assembly.services` 绑定 `dyn ScriptExecutor`，保留 `ScriptRuntimePlugin`，省略对应默认语言插件。执行器具体类型仍是 TypeId 身份，语言只用于检查候选输入格式；同一类型重复绑定被拒绝，同一语言匹配多个实现时拒绝编译，不按注册顺序覆盖。

`EnhancementServicePlugin` 将此运行时用于候选编译、样例重放与持久版本恢复，编译产物用于在线调用。自定义原生执行器属于可信宿主代码，必须自行保持状态隔离、资源上限与取消约定；脚本无法替换执行器或扩大权限。增强服务统一保留输入输出配额、并发额度、租户授权和原子版本激活。

只有 `ScriptRuntimePlugin` 和语言插件的宿主可以使用无效数据库配置启动并执行纯计算。增加 `EnhancementServicePlugin` 后才需要数据库；MCP/gRPC 直接依赖此服务，不再通过组件资源间接初始化执行引擎。Wasm 与 Sidecar 的 v2 整包执行统一使用 `ComponentExecutor`，同样不要求选择默认实现。

## 整包执行器

从 `component_execution.rs` 的装配入口开始，再读 `components/execution.rs` 的契约与 `components/README.md` 的数据流。保留 `ComponentExecutionPlugin` / `ComponentResourcesPlugin` / `ComponentsPlugin`，省略 `WasmComponentPlugin` 或 `SidecarPlugin`，在自己的插件中绑定 `dyn ComponentExecutor`。同一具体类型重复绑定、没有匹配和多个匹配都明确失败；`supports` 只匹配已验证的整包格式，不是运行时身份。

安装记录是版本提交点。执行器负责准备并激活实例、按指定已提交整包恢复、释放单实例及关闭本次宿主资源。候选激活失败或安装 SQL 提交失败时，宿主清理异类候选并恢复旧执行器；一个清理失败不跳过其他补偿。提交成功后的旧实例清理失败会被报告，不会把已成功提交的安装误报为失败。关闭保留安装状态和活动版本；初始化失败或 panic、关闭 panic 均经过资源补偿。原生实现必须能关闭部分初始化的资源。

`ExecutionContext` 复用租户资源与宿主同一个 Dill 容器；它只交给可信的原生执行器。Rhai、TypeScript、Wasm 和 Sidecar 内的业务代码不能通过这个接口突破能力授权。`TypeId` 不进入数据库；重启时从安装整包重新选择执行器，在线请求仍核对租户与已提交摘要。

## 验证

`plugins::tests` 使用无效数据库地址启动真实 HTTP 入口，验证仅安装自定义路由时没有隐式资源；`plugins::infrastructure_tests` 验证注入共享、初始化顺序、无业务恢复时资源仍就绪、失败与 panic 后补偿、重复资源及歧义 Provider 拒绝。独立 PostgreSQL 测试验证替换 Provider 后的迁移、查询和连接池关闭。协议与 Sidecar 测试复用正式装配，验证所有调用通道继续共享增强版本和授权。

外部 crate 集成测试 `tests/component_executors.rs` 只消费公开契约，覆盖实际 HTTP 发布、安装、挂载、调用、不同执行器切换、回滚、真实 COMMIT 失败、停用、卸载和重启恢复；还覆盖缺失/歧义匹配、重复类型、初始化失败、panic、取消等待和关闭错误的补偿。默认 Wasm/Sidecar 回归使用真实 WIT 组件和 Unix socket。
