//! 默认持久化与后台调度由独立插件拥有；公共 HTTP 插件只依赖接口。
use super::super::delivery::{
    DeliveryService, DeliverySources, PluginDocumentation, postgres::PostgresDelivery,
    postgres_documents::PostgresDocumentation, postgres_sources::PostgresDeliverySources,
};
use super::{ComponentsPlugin, ManagementPlugin, Resource, RuntimeAssembly, StorePlugin};
use az_plugin_core::{Plugin, PluginFuture};
use std::{any::TypeId, sync::Arc};

/// 正式文档后端使用既有目录表；可以只装配文档，不启动构建队列或发现任务。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct PostgresDocumentationPlugin;
impl Plugin<RuntimeAssembly> for PostgresDocumentationPlugin {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![TypeId::of::<StorePlugin>()]
    }
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.require_service::<sqlx::PgPool>();
            assembly.services.add::<PostgresDocumentation>();
            assembly.require_service::<dyn PluginDocumentation>();
            Ok(())
        })
    }
}

/// PostgreSQL 交付协调器保留跨目录事务；发现和自动升级由各自插件启动。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct PostgresDeliveryPlugin;
impl Plugin<RuntimeAssembly> for PostgresDeliveryPlugin {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![
            TypeId::of::<StorePlugin>(),
            TypeId::of::<ManagementPlugin>(),
            TypeId::of::<ComponentsPlugin>(),
        ]
    }
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.require_service::<dyn PluginDocumentation>();
            assembly.services.add::<PostgresDeliverySources>();
            assembly.require_service::<dyn DeliverySources>();
            assembly.declare_resource::<PostgresDelivery>()?;
            assembly
                .services
                .bind::<dyn DeliveryService, PostgresDelivery>();
            assembly.require_service::<dyn DeliveryService>();
            // 服务和后台工作持有当前代的弱引用，组合变化后必须重建。
            assembly.observe_catalog();
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    let service = Arc::new(PostgresDelivery::new(&state)?);
                    state
                        .service::<Resource<PostgresDelivery>>()?
                        .provide(service)
                })
            });
            assembly.on_ready::<Self>(|state, tasks| {
                Box::pin(async move { super::super::delivery::start_publication(state, tasks) })
            });
            Ok(())
        })
    }
}

/// GitHub 发现器只依赖来源契约；可独立移除或使用替代队列，默认访问公共 GitHub。
/// api_base 由部署方指定，可使用企业 GitHub 的 /api/v3/，不开放给业务请求。
pub struct GithubDeliveryDiscoveryPlugin {
    pub api_base: String,
}
impl Default for GithubDeliveryDiscoveryPlugin {
    fn default() -> Self {
        Self {
            api_base: "https://api.github.com/".into(),
        }
    }
}
impl Plugin<RuntimeAssembly> for GithubDeliveryDiscoveryPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            let Some(config) = assembly.config.delivery.clone() else {
                return Ok(());
            };
            assembly.require_service::<dyn DeliverySources>();
            let discovery =
                super::super::delivery::discovery::GithubDiscovery::new(&self.api_base, config)?;
            assembly.services.add_value(discovery);
            assembly.on_ready::<Self>(|state, tasks| {
                Box::pin(async move {
                    let discovery =
                        state.service::<super::super::delivery::discovery::GithubDiscovery>()?;
                    let sources = state.service::<dyn DeliverySources>()?;
                    tasks.spawn(discovery.run(sources, tasks.cancellation()))
                })
            });
            Ok(())
        })
    }
}

/// 默认自动升级策略独立持有定时任务，移除它不影响构建领取、上传和完成提交。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct PostgresDeliveryRolloutPlugin;
impl Plugin<RuntimeAssembly> for PostgresDeliveryRolloutPlugin {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![
            TypeId::of::<StorePlugin>(),
            TypeId::of::<ManagementPlugin>(),
            TypeId::of::<ComponentsPlugin>(),
        ]
    }
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.observe_catalog();
            assembly.on_ready::<Self>(|state, tasks| {
                Box::pin(async move { super::super::delivery::start_rollout(state, tasks) })
            });
            Ok(())
        })
    }
}
