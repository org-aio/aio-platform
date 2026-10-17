#[cfg(all(feature = "postgres", any(feature = "wasm", feature = "sidecar")))]
use super::{component_execution::*, component_resources::*};
use std::any::TypeId;

#[cfg(feature = "postgres")]
use anyhow::Result;
#[cfg(feature = "postgres")]
use az_plugin_core::DynPlugin;
use az_plugin_core::{Plugin, PluginFuture};
#[cfg(feature = "postgres")]
use dill::CatalogBuilder;

use super::super::plugin_catalog::PluginCatalog;
use super::{
    CapabilityServicePlugin, RuntimeAssembly, enhancement_execution::*, infrastructure::*,
};
#[cfg(feature = "postgres")]
use super::{platform_services::*, postgres::*};
use crate::generated::{clipboard, personal_config, worker, worker_webview};

// 每项能力声明自己的服务、路由和具体类型依赖；宏只展开同一套注册步骤。
macro_rules! route_plugin {
    ($name:ident, [$($dependency:ty),*], [$($required:ty),*], $(optional [$($optional:ty),*],)? |$state:pat_param| $routes:expr) => {
        /// 向宿主贡献该能力的服务与路由，由应用装配层决定是否安装。
        #[dill::component]
        #[dill::interface(dyn Plugin<RuntimeAssembly>)]
        pub struct $name;

        impl Plugin<RuntimeAssembly> for $name {
            fn dependencies(&self) -> Vec<TypeId> {
                vec![$(TypeId::of::<$dependency>()),*]
            }

            fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
                Box::pin(async move {
                    assembly.require_service::<dyn crate::identity::IdentityProvider>();
                    $(assembly.require_service::<$required>();)*
                    $($(assembly.optional_service::<$optional>();)*)?
                    assembly.add_routes(|$state| Ok($routes));
                    Ok(())
                })
            }
        }
    };
}

route_plugin!(
    FrontendPlugin,
    [RepositoryPlugin],
    [dyn PluginCatalog],
    |_| super::super::routes::frontend()
);
route_plugin!(EnhancementsPlugin, [EnhancementServicePlugin], [], |_| {
    super::super::enhancements::router()
});
route_plugin!(CapabilitiesPlugin, [CapabilityServicePlugin], [], |_| {
    super::super::capabilities::router()
});
#[cfg(feature = "mcp")]
route_plugin!(
    McpPlugin,
    [CapabilityServicePlugin],
    [],
    optional[super::super::enhancements::Enhancements],
    |state| super::super::protocols::mcp::router(state.clone())
);
#[cfg(feature = "grpc")]
route_plugin!(
    GrpcPlugin,
    [CapabilityServicePlugin],
    [],
    optional[super::super::enhancements::Enhancements],
    |state| super::super::protocols::grpc::router(state.clone())?
);
route_plugin!(BootstrapPlugin, [], [dyn PluginCatalog], |_| {
    super::super::bootstrap::router()
});
route_plugin!(NavigationPlugin, [], [dyn PluginCatalog], |_| {
    super::super::navigation::router()
});
/// 默认工具业务跨表查询，显式依赖 PostgreSQL 目录提供者。
#[cfg(feature = "postgres")]
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct ToolsPlugin;
#[cfg(feature = "postgres")]
impl Plugin<RuntimeAssembly> for ToolsPlugin {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![
            TypeId::of::<StorePlugin>(),
            TypeId::of::<RepositoryPlugin>(),
        ]
    }
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.require_service::<dyn crate::identity::IdentityProvider>();
            assembly.require_service::<dyn crate::authorization::AuthorizationProvider>();
            assembly.require_service::<dyn worker::WorkerService>();
            assembly
                .services
                .add::<super::super::tools::PostgresToolCatalog>();
            assembly.require_service::<dyn super::super::tools::ToolCatalog>();
            assembly.add_routes(|_| Ok(super::super::tools::router()));
            Ok(())
        })
    }
}
route_plugin!(WorkersPlugin, [], [dyn worker::WorkerService], |_| {
    worker::controller::router()
});
/// 设备网页入口消费由插件贡献的协议；汇集、校验和在线依赖重建仍使用 Dill。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct WorkerWebviewsPlugin;

impl Plugin<RuntimeAssembly> for WorkerWebviewsPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.require_service::<dyn crate::identity::IdentityProvider>();
            assembly.require_service::<dyn worker::WorkerService>();
            assembly.require_service::<dyn worker_webview::WorkerWebviewService>();
            assembly.services.add::<worker_webview::WebviewProtocols>();
            assembly.require_service::<worker_webview::WebviewProtocols>();
            // 即使没有启动钩子，长连接也需要随协议提供者变更而取消的作用域。
            assembly.hooks::<Self>();
            assembly.requirements.push(|services, resolve| {
                if resolve {
                    services
                        .get_one::<worker_webview::WebviewProtocols>()?
                        .validate()?;
                }
                Ok(())
            });
            assembly.add_routes(|_| Ok(worker_webview::controller::router()));
            Ok(())
        })
    }
}
route_plugin!(PersonalConfigPlugin, [], [dyn personal_config::PersonalConfigService, dyn worker::WorkerService], |state| personal_config::controller::router(state.clone()));
route_plugin!(ClipboardPlugin, [], [dyn clipboard::ClipboardService, dyn worker::WorkerService], |state| clipboard::controller::router(state.clone()));

/// 管理插件拥有默认包恢复、旧执行器恢复与发布任务恢复。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct ManagementPlugin;

impl Plugin<RuntimeAssembly> for ManagementPlugin {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![TypeId::of::<RepositoryPlugin>()]
    }
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.require_service::<dyn crate::identity::IdentityProvider>();
            assembly.require_service::<dyn crate::authorization::AuthorizationProvider>();
            assembly.require_service::<dyn PluginCatalog>();
            assembly.optional_service::<dyn super::super::tools::ToolCatalog>();
            // 发布任务按安装包动态选择执行器，持有的运行时快照必须随组合更新。
            assembly.observe_catalog();
            assembly.add_routes(|state| Ok(super::super::routes::management(state.clone())));
            assembly.on_start::<Self>(|state, _tasks| {
                Box::pin(async move {
                    state.ensure_default_plugins().await?;
                    state.reconcile_wasm().await?;
                    state.reconcile_processes().await
                })
            });
            assembly.on_ready::<Self>(|state, _tasks| {
                Box::pin(async move { state.resume_published_jobs().await })
            });
            Ok(())
        })
    }
}

/// 组件插件恢复已安装的 Wasm 与 sidecar 版本。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct ComponentsPlugin;

impl Plugin<RuntimeAssembly> for ComponentsPlugin {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![TypeId::of::<ComponentResourcesPlugin>()]
    }
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.require_service::<dyn crate::identity::IdentityProvider>();
            assembly.require_service::<dyn crate::authorization::AuthorizationProvider>();
            assembly.add_routes(|state| Ok(super::super::components::router(state.clone())));
            assembly.on_start::<Self>(|state, _tasks| {
                Box::pin(async move {
                    if let Some(components) = state
                        .optional_service::<crate::runtime::server::components::Components>()?
                    {
                        components.restore().await?;
                    }
                    Ok(())
                })
            });
            Ok(())
        })
    }
}

route_plugin!(DeliveryPlugin, [], [dyn super::super::delivery::DeliveryService, dyn crate::authorization::AuthorizationProvider], |_| super::super::delivery::router());
route_plugin!(DocumentationPlugin, [], [], optional[dyn super::super::delivery::PluginDocumentation, dyn super::super::delivery::DeliveryService, super::super::components::Components], |_| super::super::delivery::documents::router());

/// 默认平台组合；精简宿主或定制宿主可以从自己的 Dill Catalog 解析其他组合。
#[cfg(feature = "postgres")]
pub fn default_plugins() -> Result<Vec<DynPlugin<RuntimeAssembly>>> {
    let mut catalog = CatalogBuilder::new();
    catalog
        .add::<PostgresPlugin>()
        .add::<DatabasePlugin>()
        .add::<FileKeyringPlugin>()
        .add::<KeyringPlugin>()
        .add::<StorePlugin>()
        .add::<RepositoryPlugin>()
        .add::<CapabilityServicePlugin>()
        .add::<CapabilitiesPlugin>()
        .add::<WorkerServicePlugin>()
        .add::<super::WorkerCapabilitiesPlugin>()
        .add::<WorkerWebviewServicePlugin>()
        .add::<PersonalConfigServicePlugin>()
        .add::<ClipboardServicePlugin>()
        .add::<ManagementPlugin>()
        .add::<FrontendPlugin>()
        .add::<BootstrapPlugin>()
        .add::<NavigationPlugin>()
        .add::<super::PostgresDocumentationPlugin>()
        .add::<DocumentationPlugin>()
        .add::<ToolsPlugin>()
        .add::<WorkersPlugin>()
        .add::<WorkerWebviewsPlugin>()
        .add::<PersonalConfigPlugin>()
        .add::<ClipboardPlugin>();
    #[cfg(feature = "rhai")]
    catalog.add::<RhaiPlugin>();
    #[cfg(feature = "typescript")]
    catalog.add::<TypeScriptPlugin>();
    // 仅装配协议时暴露原生能力；脚本版本服务由所选执行器独立引入。
    #[cfg(any(feature = "rhai", feature = "typescript"))]
    catalog
        .add::<ScriptRuntimePlugin>()
        .add::<super::PostgresEnhancementCatalogPlugin>()
        .add::<EnhancementServicePlugin>()
        .add::<super::EvolutionPlugin>()
        .add::<EnhancementsPlugin>();
    #[cfg(feature = "mcp")]
    catalog.add::<McpPlugin>();
    #[cfg(feature = "grpc")]
    catalog.add::<GrpcPlugin>();
    #[cfg(feature = "wasm")]
    catalog.add::<WasmPlugin>().add::<WasmComponentPlugin>();
    #[cfg(feature = "sidecar")]
    catalog.add::<ProcessPlugin>().add::<SidecarPlugin>();
    #[cfg(any(feature = "remote-mcp", feature = "remote-grpc"))]
    catalog.add::<super::RemoteCapabilitiesPlugin>();
    #[cfg(any(feature = "wasm", feature = "sidecar"))]
    catalog
        .add::<super::PostgresComponentCatalogPlugin>()
        .add::<PostgresComponentPlugin>()
        .add::<FileObjectStoragePlugin>()
        .add::<ComponentExecutionPlugin>()
        .add::<ComponentResourcesPlugin>()
        .add::<ComponentsPlugin>()
        .add::<super::PostgresDeliveryPlugin>()
        .add::<super::PostgresDeliveryRolloutPlugin>()
        .add_value(super::GithubDeliveryDiscoveryPlugin::default())
        .bind::<dyn Plugin<RuntimeAssembly>, super::GithubDeliveryDiscoveryPlugin>()
        .add::<DeliveryPlugin>();
    let catalog = catalog.build();
    Ok(catalog
        .get::<dill::AllOf<dyn Plugin<RuntimeAssembly>>>()?
        .to_vec())
}
