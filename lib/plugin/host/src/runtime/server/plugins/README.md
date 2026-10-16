# 宿主能力插件

宿主通过 `Plugin<RuntimeAssembly>` 复用 `az-plugin-core` 的 TypeId 身份和依赖排序，通过 Dill 提供服务。`RuntimeState` 不再拥有剪贴板、个人配置、设备和设备视图的固定服务字段；HTTP、组件 broker 和 sidecar 使用同一个服务容器。

先看 `RuntimeAssembly`，再看 `builtins.rs` 中的默认平台组合。插件可以注册服务、贡献 HTTP 路由，以及注册恢复与就绪动作。所有恢复动作成功后才执行就绪动作；省略交付插件也会省略它的后台启动动作。`require_service::<dyn Trait>()` 检查接口的唯一绑定，允许应用提供其他实现。`Plugin::dependencies()` 只声明必须先构建的具体插件类型，不能用它强制绑定某个默认服务实现。

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

插件的 `build` 负责声明装配，不应启动后台任务或执行迁移。将异步恢复交给 `on_start`，将后台工作交给 `on_ready`。路由通过 `state.service::<dyn MyService>()?` 获取服务。长连接在升级前解析服务，并把同一个 `Arc` 传入会话；身份与撤权检查仍在会话期间持续执行。

这是原生宿主插件的装配边界，还不包含在线卸载、启动失败后的完整资源回收或脚本服务代理。HTTP 路由不能替代能力授权。数据库连接、执行器创建与部分组件内部装配仍待迁移，完整目标见 [一切皆插件验收清单](../../../../../../../docs/plugin/everything-as-plugin.md)。

验证：`cargo test -p az-plugin-host --features server plugins::tests`。真实 HTTP 组合验收还需要设置 `AIO_ENHANCEMENT_TEST_DATABASE_URL` 后添加 `-- --ignored`。
