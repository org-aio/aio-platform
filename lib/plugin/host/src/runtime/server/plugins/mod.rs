//! 宿主能力装配入口；插件统一复用核心的 TypeId 身份、依赖排序和 Dill。

mod admission;
mod builtins;
mod capability_execution;
mod component_bootstrap;
mod component_execution;
mod component_management;
mod component_resources;
mod composition;
mod enhancement_execution;
mod evolution;
#[cfg(test)]
mod identity_tests;
mod infrastructure;
#[cfg(test)]
mod infrastructure_tests;
mod lifecycle;
#[cfg(test)]
mod lifecycle_tests;
mod live;
#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod optional_tests;
pub(super) mod process_execution;
#[cfg(any(feature = "remote-mcp", feature = "remote-grpc"))]
mod remote_capabilities;
mod resource;
mod service_bindings;
mod tasks;
#[cfg(test)]
mod tests;
mod wasm_execution;
mod worker_capabilities;

use std::{any::TypeId, future::Future, pin::Pin, sync::Arc};

use anyhow::{Context, Result};
use axum::Router;
use az_plugin_core::{DynPlugin, Plugin};
use dill::Catalog;

use super::RuntimeState;

pub(super) use admission::{admit, admit_router};
pub use builtins::*;
pub use capability_execution::*;
pub use component_bootstrap::{BootstrapComponent, ComponentBootstrapPlugin};
pub use component_execution::*;
pub(in crate::runtime::server) use component_management::ManagementAccess;
pub use component_management::{ComponentManagementPlugin, TrustedManagementComponent};
pub use component_resources::*;
pub use enhancement_execution::*;
pub use evolution::EvolutionPlugin;
pub use infrastructure::*;
use lifecycle::{HookPlan, Lifecycle};
pub use live::RuntimeHost;
pub use process_execution::{
    ProcessExecutor, ProcessInstance, ProcessResponse, StartProcessRequest,
};
#[cfg(any(feature = "remote-mcp", feature = "remote-grpc"))]
pub use remote_capabilities::RemoteCapabilitiesPlugin;
pub use resource::Resource;
pub use service_bindings::ServiceBindings;
pub use tasks::PluginTasks;
pub use wasm_execution::{WasmActivation, WasmArtifactValidator, WasmExecutor};
pub use worker_capabilities::WorkerCapabilitiesPlugin;

/// 插件贡献的 HTTP 路由；认证仍由各能力的正式入口执行。
pub type RouteFactory = fn(&RuntimeState) -> Result<Router<RuntimeState>>;

/// 插件的启动、就绪或停止动作；任务作用域按具体插件类型共享。
pub type LifecycleHook =
    fn(RuntimeState, Arc<PluginTasks>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>;

/// 启动插件只向现有 Dill 容器和路由集合贡献能力，不创建第二套服务注册表。
#[derive(Clone)]
pub struct RuntimeAssembly {
    pub services: ServiceBindings,
    pub config: Arc<crate::configuration::HostConfig>,
    routes: Vec<RouteFactory>,
    requirements: Vec<fn(&Catalog, bool) -> Result<()>>,
    lifecycle: Vec<HookPlan>,
    service_dependencies: std::collections::HashSet<TypeId>,
    catalog_sensitive: bool,
}

impl RuntimeAssembly {
    /// 为当前插件添加路由。未安装该插件时不会暴露这些入口。
    pub fn add_routes(&mut self, routes: RouteFactory) {
        self.routes.push(routes);
    }

    /// 声明异步资源；业务服务直接注入 T，初始化钩子通过 Resource<T> 提供实例。
    pub fn declare_resource<T: Send + Sync + 'static>(&mut self) -> Result<()> {
        resource::declare::<T>(&mut self.services)
    }

    /// 初始化插件资源；开发宿主同样执行，全部成功后才运行恢复与后台钩子。
    pub fn on_initialize<P: Plugin<Self>>(&mut self, hook: LifecycleHook) {
        self.hooks::<P>().initialize.push(hook);
    }

    /// 恢复插件拥有的状态；停止动作在调用前已经登记，必须能处理部分启动。
    pub fn on_start<P: Plugin<Self>>(&mut self, hook: LifecycleHook) {
        self.hooks::<P>().start.push(hook);
    }

    /// 全部恢复成功后才开始后台工作；后台任务必须交给传入的任务作用域。
    pub fn on_ready<P: Plugin<Self>>(&mut self, hook: LifecycleHook) {
        self.hooks::<P>().ready.push(hook);
    }

    /// 按依赖反序释放本次宿主持有的资源，不删除持久化安装状态。
    pub fn on_stop<P: Plugin<Self>>(&mut self, hook: LifecycleHook) {
        self.hooks::<P>().stop.push(hook);
    }

    fn hooks<P: Plugin<Self>>(&mut self) -> &mut HookPlan {
        let owner = TypeId::of::<P>();
        let index = match self.lifecycle.iter().position(|plan| plan.owner == owner) {
            Some(index) => index,
            None => {
                self.lifecycle.push(HookPlan {
                    owner,
                    initialize: Vec::new(),
                    start: Vec::new(),
                    ready: Vec::new(),
                    stop: Vec::new(),
                });
                self.lifecycle.len() - 1
            }
        };
        &mut self.lifecycle[index]
    }

    /// 保存整个容器或其弱引用的服务必须声明此项；任何组合变更都重建该插件。
    pub fn observe_catalog(&mut self) {
        self.catalog_sensitive = true;
    }

    /// 可选服务存在时必须唯一，并先于使用方初始化；缺省时不创建替代实现。
    /// 在线增删提供者同样触发依赖重建，实际解析仍使用同一个 Dill 容器。
    pub fn optional_service<I: ?Sized + Send + Sync + 'static>(&mut self) {
        self.service_dependencies.insert(TypeId::of::<I>());
        self.requirements.push(|catalog, resolve| {
            let count = catalog.builders_for::<I>().count();
            anyhow::ensure!(count <= 1, "可选服务存在重复绑定: {:?}", TypeId::of::<I>());
            if resolve && count == 1 {
                catalog.get_one::<I>().context("初始化可选宿主服务失败")?;
            }
            Ok(())
        });
    }

    /// 路由依赖接口而不是某个默认实现；所有插件注册完成后检查唯一绑定。
    pub fn require_service<I: ?Sized + Send + Sync + 'static>(&mut self) {
        self.service_dependencies.insert(TypeId::of::<I>());
        self.requirements.push(|catalog, resolve| {
            anyhow::ensure!(
                catalog.builders_for::<I>().count() == 1,
                "宿主插件缺少唯一的服务绑定: {:?}",
                TypeId::of::<I>()
            );
            if resolve {
                catalog.get_one::<I>().context("初始化宿主服务失败")?;
            }
            Ok(())
        });
    }
}

/// 已通过依赖校验的运行时能力集合；所有调用通道共享同一个服务容器。
pub(crate) struct RuntimeExtensions {
    pub services: Catalog,
    pub routes: Vec<RouteFactory>,
    lifecycle: Lifecycle,
    requirements: Vec<fn(&Catalog, bool) -> Result<()>>,
    contributions: Vec<composition::Contribution>,
}

impl RuntimeExtensions {
    pub(super) async fn start(&self, state: &RuntimeState, run_hooks: bool) -> Result<()> {
        Lifecycle::start(state, run_hooks).await
    }
}

impl RuntimeState {
    /// 按插件的 TypeId 获取相同的任务作用域，供请求入口创建受管理的工作。
    pub fn plugin_tasks<P: Plugin<RuntimeAssembly>>(&self) -> Result<Arc<PluginTasks>> {
        self.extensions.lifecycle.tasks(TypeId::of::<P>())
    }

    /// 停止接收运行时请求，等待正在提交的工作，再反序释放资源；可重复调用。
    /// 停止是协作式的，插件必须在安全边界响应取消；调用方也应停止监听器。
    pub async fn shutdown(&self) -> Result<()> {
        self.extensions.lifecycle.cancellation.cancel();
        let state = self.clone();
        tokio::spawn(async move { state.extensions.lifecycle.stop(&state).await })
            .await
            .context("插件停止任务未正常结束")?
    }
}

pub(super) async fn assemble(
    config: Arc<crate::configuration::HostConfig>,
    plugins: Vec<DynPlugin<RuntimeAssembly>>,
) -> Result<Arc<RuntimeExtensions>> {
    validate_selection(&config, &plugins)?;
    let contributions = composition::build(config, plugins, &[], &Default::default()).await?;
    composition::assemble(contributions, None, &Default::default())
}

fn validate_selection(
    config: &crate::configuration::HostConfig,
    plugins: &[DynPlugin<RuntimeAssembly>],
) -> Result<()> {
    if config.remote_capabilities.is_some() {
        #[cfg(not(any(feature = "remote-mcp", feature = "remote-grpc")))]
        anyhow::bail!("远端能力配置需要启用 remote-mcp 或 remote-grpc feature");
        #[cfg(any(feature = "remote-mcp", feature = "remote-grpc"))]
        anyhow::ensure!(plugins.iter().any(|plugin| plugin.as_ref().type_id() == TypeId::of::<RemoteCapabilitiesPlugin>()), "远端能力配置需要装配 RemoteCapabilitiesPlugin");
    }
    let _ = plugins;
    Ok(())
}

// 身份相关服务必须在所有入口共享同一实例，不能每次解析生成新对象。
fn validate_security_provider<I: ?Sized + 'static>(services: &Catalog) -> Result<()> {
    use dill::Builder;
    let mut bindings = services.builders_for::<I>();
    let Some(binding) = bindings.next() else {
        return Ok(());
    };
    anyhow::ensure!(
        bindings.next().is_none(),
        "身份、授权或计费接口存在重复提供者: {:?}",
        TypeId::of::<I>()
    );
    anyhow::ensure!(
        binding.scope_type().id == TypeId::of::<dill::Singleton>(),
        "身份、授权或计费提供者必须声明 Dill 单例: {:?}",
        TypeId::of::<I>()
    );
    Ok(())
}
