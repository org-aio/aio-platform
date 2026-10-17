//! 默认持久化与后台调度由独立插件拥有；公共 HTTP 插件只依赖接口。
use super::super::delivery::{
    DeliveryService, PluginDocumentation, postgres::PostgresDelivery,
    postgres_documents::PostgresDocumentation,
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

/// PostgreSQL 交付协调器保留跨目录事务，并在所有恢复成功后启动发现与滚动发布。
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
                Box::pin(async move { super::super::delivery::start(state, tasks) })
            });
            Ok(())
        })
    }
}
