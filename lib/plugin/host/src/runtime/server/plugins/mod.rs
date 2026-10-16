//! 宿主能力装配入口；插件统一复用核心的 TypeId 身份、依赖排序和 Dill。

mod builtins;
#[cfg(test)]
mod tests;

use std::{future::Future, pin::Pin, sync::Arc};

use anyhow::{Context, Result};
use axum::Router;
use az_plugin_core::{App, DynPlugin};
use az_plugin_runtime::Keyring;
use dill::{Catalog, CatalogBuilder};
use sqlx::PgPool;

use super::RuntimeState;

pub use builtins::*;

/// 插件贡献的 HTTP 路由；认证仍由各能力的正式入口执行。
pub type RouteFactory = fn(&RuntimeState) -> Router<RuntimeState>;

/// 插件的异步恢复动作；基础设施已连接，但宿主还未开始接受请求。
pub type StartupHook = fn(RuntimeState) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>;

/// 启动插件只向现有 Dill 容器和路由集合贡献能力，不创建第二套服务注册表。
pub struct RuntimeAssembly {
    pub services: CatalogBuilder,
    routes: Vec<RouteFactory>,
    requirements: Vec<fn(&Catalog) -> Result<()>>,
    startup: Vec<StartupHook>,
    ready: Vec<StartupHook>,
}

impl RuntimeAssembly {
    /// 为当前插件添加路由。未安装该插件时不会暴露这些入口。
    pub fn add_routes(&mut self, routes: RouteFactory) {
        self.routes.push(routes);
    }

    /// 恢复该插件拥有的持久状态，按插件依赖顺序执行。
    pub fn on_start(&mut self, hook: StartupHook) {
        self.startup.push(hook);
    }

    /// 所有恢复动作成功后才开始后台工作，避免在依赖未就绪时接收任务。
    pub fn on_ready(&mut self, hook: StartupHook) {
        self.ready.push(hook);
    }

    /// 路由依赖接口而不是某个默认实现；所有插件注册完成后检查唯一绑定。
    pub fn require_service<I: ?Sized + Send + Sync + 'static>(&mut self) {
        self.requirements.push(|catalog| {
            catalog
                .get_one::<I>()
                .context("宿主插件缺少唯一的服务绑定")?;
            Ok(())
        });
    }
}

/// 已通过依赖校验的运行时能力集合；所有调用通道共享同一个服务容器。
pub(crate) struct RuntimeExtensions {
    pub services: Catalog,
    pub routes: Vec<RouteFactory>,
    startup: Vec<StartupHook>,
    ready: Vec<StartupHook>,
}

impl RuntimeExtensions {
    pub(super) async fn start(&self, state: &RuntimeState) -> Result<()> {
        for hook in self.startup.iter().chain(&self.ready) {
            hook(state.clone()).await?;
        }
        Ok(())
    }
}

pub(super) async fn assemble(
    pool: PgPool,
    keyring: Keyring,
    plugins: Vec<DynPlugin<RuntimeAssembly>>,
) -> Result<Arc<RuntimeExtensions>> {
    let mut services = CatalogBuilder::new();
    services.add_value(pool).add_value(keyring);
    let mut app = App::new(RuntimeAssembly {
        services,
        routes: Vec::new(),
        requirements: Vec::new(),
        startup: Vec::new(),
        ready: Vec::new(),
    });
    app.add_plugins(plugins).await?;
    let mut assembly = app.into_inner();
    assembly
        .services
        .validate()
        .context("校验宿主能力依赖失败")?;
    let services = assembly.services.build();
    for validate in assembly.requirements {
        validate(&services)?;
    }
    Ok(Arc::new(RuntimeExtensions {
        services,
        routes: assembly.routes,
        startup: assembly.startup,
        ready: assembly.ready,
    }))
}
