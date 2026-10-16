# 宿主能力插件

宿主通过 `Plugin<RuntimeAssembly>` 复用 `az-plugin-core` 的 TypeId 身份和依赖排序，通过 Dill 提供服务。`RuntimeState` 不再拥有剪贴板、个人配置、设备和设备视图的固定服务字段；HTTP、组件 broker 和 sidecar 使用同一个服务容器。

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

插件的 `build` 负责声明装配，不应启动后台任务或执行迁移。将异步恢复交给 `on_start::<Self>`，将后台工作交给 `on_ready::<Self>`，将资源清理交给 `on_stop::<Self>`。路由通过 `state.service::<dyn MyService>()?` 获取服务。长连接在升级前解析服务，并把同一个 `Arc` 传入会话；身份与撤权检查仍在会话期间持续执行。

生命周期顺序来自内核的 `App::plugin_order()`，身份始终是具体类型的 `TypeId`。进入插件的启动动作前登记停止动作，因此停止回调必须能处理部分初始化，也不能删除安装记录或已经提交的业务数据。所有启动动作成功后才执行就绪动作；失败、panic 或取消会先停止接收新任务、等待已有工作完成，再按依赖反序执行停止回调。一个回调失败不会跳过其他插件，错误会汇总返回；重复关闭不重复执行回调，并保留第一次的结果。

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

这是原生宿主插件的装配边界，还不包含在线卸载、单插件重启或脚本服务代理。HTTP 路由不能替代能力授权。数据库连接、执行器创建与部分组件内部装配仍待迁移，完整目标见 [一切皆插件验收清单](../../../../../../../docs/plugin/everything-as-plugin.md)。

验证：`cargo test -p az-plugin-host --features server plugins::tests`。真实 HTTP 组合验收还需要设置 `AIO_ENHANCEMENT_TEST_DATABASE_URL` 后添加 `-- --ignored`。

故障验收：`cargo test -p az-plugin-host --features server lifecycle_tests` 覆盖真实端口释放、启动/就绪失败、取消等待、回调 panic、后台工作提交与 HTTP 收尾。`process::shutdown_tests` 使用真实 Unix socket 监督器协议验证只停止本次创建的进程；Component 的持久化停止验收复用 `publication_preserves_running_singleton_process`。
