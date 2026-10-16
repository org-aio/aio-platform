//! 宿主能力装配入口；插件统一复用核心的 TypeId 身份、依赖排序和 Dill。

mod admission;
mod builtins;
mod component_execution;
mod enhancement_execution;
mod infrastructure;
#[cfg(test)]
mod infrastructure_tests;
mod lifecycle;
#[cfg(test)]
mod lifecycle_tests;
mod resource;
mod tasks;
#[cfg(test)]
mod tests;

use std::{any::TypeId, future::Future, pin::Pin, sync::Arc};

use anyhow::{Context, Result};
use axum::Router;
use az_plugin_core::{App, DynPlugin, Plugin};
use dill::{Catalog, CatalogBuilder};

use super::RuntimeState;

pub(super) use admission::admit;
pub use builtins::*;
pub use component_execution::*;
pub use enhancement_execution::*;
pub use infrastructure::*;
use lifecycle::{HookPlan, Lifecycle};
pub use resource::Resource;
pub use tasks::PluginTasks;

/// 插件贡献的 HTTP 路由；认证仍由各能力的正式入口执行。
pub type RouteFactory = fn(&RuntimeState) -> Router<RuntimeState>;

/// 插件的启动、就绪或停止动作；任务作用域按具体插件类型共享。
pub type LifecycleHook =
    fn(RuntimeState, Arc<PluginTasks>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>;

/// 启动插件只向现有 Dill 容器和路由集合贡献能力，不创建第二套服务注册表。
pub struct RuntimeAssembly {
    pub services: CatalogBuilder,
    pub config: Arc<crate::configuration::HostConfig>,
    routes: Vec<RouteFactory>,
    requirements: Vec<fn(&Catalog, bool) -> Result<()>>,
    lifecycle: Vec<HookPlan>,
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

    /// 路由依赖接口而不是某个默认实现；所有插件注册完成后检查唯一绑定。
    pub fn require_service<I: ?Sized + Send + Sync + 'static>(&mut self) {
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
    let services = CatalogBuilder::new();
    let mut app = App::new(RuntimeAssembly {
        services,
        config,
        routes: Vec::new(),
        requirements: Vec::new(),
        lifecycle: Vec::new(),
    });
    app.add_plugins(plugins).await?;
    let installed = app.plugin_order().to_vec();
    let mut assembly = app.into_inner();
    anyhow::ensure!(
        assembly
            .lifecycle
            .iter()
            .all(|plan| installed.contains(&plan.owner)),
        "生命周期声明引用了未安装的插件类型"
    );
    assembly
        .lifecycle
        .sort_by_key(|plan| installed.iter().position(|owner| *owner == plan.owner));
    assembly
        .services
        .validate()
        .context("校验宿主能力依赖失败")?;
    let services = assembly.services.build();
    for validate in &assembly.requirements {
        validate(&services, false)?;
    }
    Ok(Arc::new(RuntimeExtensions {
        services,
        routes: assembly.routes,
        lifecycle: Lifecycle::new(assembly.lifecycle),
        requirements: assembly.requirements,
    }))
}
