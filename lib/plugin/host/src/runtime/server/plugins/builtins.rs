#[cfg(any(feature = "wasm", feature = "sidecar"))]
use super::{component_execution::*, component_resources::*};
use std::any::TypeId;

use anyhow::Result;
use az_plugin_core::{DynPlugin, Plugin, PluginFuture};
use dill::CatalogBuilder;

use super::{
    CapabilityServicePlugin, RuntimeAssembly, enhancement_execution::*, infrastructure::*,
};
use crate::generated::{clipboard, personal_config, worker, worker_webview};

// 每项能力声明自己的服务、路由和具体类型依赖；宏只展开同一套注册步骤。
macro_rules! route_plugin {
    ($name:ident, [$($dependency:ty),*], [$($required:ty),*], |$state:pat_param| $routes:expr) => {
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
                    assembly.add_routes(|$state| Ok($routes));
                    Ok(())
                })
            }
        }
    };
}

route_plugin!(FrontendPlugin, [StorePlugin, RepositoryPlugin], [], |_| {
    super::super::routes::frontend()
});
route_plugin!(EnhancementsPlugin, [EnhancementServicePlugin], [], |_| {
    super::super::enhancements::router()
});
route_plugin!(CapabilitiesPlugin, [CapabilityServicePlugin], [], |_| {
    super::super::capabilities::router()
});
#[cfg(feature = "mcp")]
route_plugin!(McpPlugin, [EnhancementServicePlugin], [], |state| {
    super::super::protocols::mcp::router(state.clone())
});
#[cfg(feature = "grpc")]
route_plugin!(GrpcPlugin, [EnhancementServicePlugin], [], |state| {
    super::super::protocols::grpc::router(state.clone())
});
route_plugin!(BootstrapPlugin, [StorePlugin], [], |_| {
    super::super::bootstrap::router()
});
route_plugin!(NavigationPlugin, [StorePlugin], [], |_| {
    super::super::navigation::router()
});
route_plugin!(
    ToolsPlugin,
    [StorePlugin, RepositoryPlugin],
    [dyn worker::WorkerService, dyn crate::authorization::AuthorizationProvider],
    |_| super::super::tools::router()
);
route_plugin!(WorkersPlugin, [], [dyn worker::WorkerService], |_| {
    worker::controller::router()
});
route_plugin!(WorkerWebviewsPlugin, [], [dyn worker_webview::WorkerWebviewService, dyn worker::WorkerService], |_| worker_webview::controller::router());
route_plugin!(PersonalConfigPlugin, [], [dyn personal_config::PersonalConfigService, dyn worker::WorkerService], |state| personal_config::controller::router(state.clone()));
route_plugin!(ClipboardPlugin, [], [dyn clipboard::ClipboardService, dyn worker::WorkerService], |state| clipboard::controller::router(state.clone()));

/// 管理插件拥有默认包恢复、旧执行器恢复与发布任务恢复。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct ManagementPlugin;

impl Plugin<RuntimeAssembly> for ManagementPlugin {
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

/// 交付插件只在完整恢复成功后启动发现和滚动发布任务。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct DeliveryPlugin;

impl Plugin<RuntimeAssembly> for DeliveryPlugin {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![
            TypeId::of::<ManagementPlugin>(),
            TypeId::of::<ComponentsPlugin>(),
        ]
    }
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.add_routes(|_| Ok(super::super::delivery::router()));
            assembly.on_ready::<Self>(|state, tasks| {
                Box::pin(async move {
                    super::super::delivery::start(state, tasks)?;
                    Ok(())
                })
            });
            Ok(())
        })
    }
}

// 默认实现独立于路由插件；应用可省略它并绑定同一接口的替代实现。
macro_rules! service_plugin {
    ($name:ident, $implementation:ty, $interface:ty, [$($dependency:ty),*]) => {
        /// 为宿主注册该服务的默认持久化实现，不绑定 HTTP 路由。
        #[dill::component]
        #[dill::interface(dyn Plugin<RuntimeAssembly>)]
        pub struct $name;
        impl Plugin<RuntimeAssembly> for $name {
            fn dependencies(&self) -> Vec<TypeId> { vec![$(TypeId::of::<$dependency>()),*] }
            fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
                Box::pin(async move {
                    assembly.services.add::<$implementation>();
                    assembly.require_service::<$interface>();
                    Ok(())
                })
            }
        }
    };
}

service_plugin!(
    WorkerServicePlugin,
    worker::WorkerServiceImpl,
    dyn worker::WorkerService,
    [StorePlugin]
);
service_plugin!(
    WorkerWebviewServicePlugin,
    worker_webview::WorkerWebviewServiceImpl,
    dyn worker_webview::WorkerWebviewService,
    [StorePlugin]
);
service_plugin!(
    PersonalConfigServicePlugin,
    personal_config::PersonalConfigServiceImpl,
    dyn personal_config::PersonalConfigService,
    [StorePlugin, KeyringPlugin]
);
service_plugin!(
    ClipboardServicePlugin,
    clipboard::ClipboardServiceImpl,
    dyn clipboard::ClipboardService,
    [StorePlugin, KeyringPlugin]
);

/// 默认平台组合；精简宿主或定制宿主可以从自己的 Dill Catalog 解析其他组合。
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
        .add::<ToolsPlugin>()
        .add::<WorkersPlugin>()
        .add::<WorkerWebviewsPlugin>()
        .add::<PersonalConfigPlugin>()
        .add::<ClipboardPlugin>();
    #[cfg(feature = "rhai")]
    catalog.add::<RhaiPlugin>();
    #[cfg(feature = "typescript")]
    catalog.add::<TypeScriptPlugin>();
    // 协议默认组合仍需执行器；只编译协议的定制宿主应显式装配外部执行器。
    #[cfg(any(
        feature = "rhai",
        feature = "typescript",
        feature = "mcp",
        feature = "grpc"
    ))]
    catalog
        .add::<ScriptRuntimePlugin>()
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
        .add::<PostgresComponentPlugin>()
        .add::<FileObjectStoragePlugin>()
        .add::<ComponentExecutionPlugin>()
        .add::<ComponentResourcesPlugin>()
        .add::<ComponentsPlugin>()
        .add::<DeliveryPlugin>();
    let catalog = catalog.build();
    Ok(catalog
        .get::<dill::AllOf<dyn Plugin<RuntimeAssembly>>>()?
        .to_vec())
}
