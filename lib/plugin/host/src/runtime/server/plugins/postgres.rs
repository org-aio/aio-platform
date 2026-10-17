//! 默认 PostgreSQL 资源与目录装配；通用宿主不编译此模块。
use super::super::store::PluginStore;
use super::{Resource, RuntimeAssembly};
use crate::configuration::HostConfig;
use anyhow::{Context, Result};
use az_plugin_core::{Plugin, PluginFuture};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    any::{Any, TypeId},
    sync::Arc,
};

/// 提供当前宿主拥有的数据库连接池；应用可替换连接建立方式，停止时统一关闭。
#[async_trait::async_trait]
pub trait DatabaseProvider: Any + Send + Sync {
    async fn connect(&self, config: &HostConfig) -> Result<PgPool>;
}

/// 默认 PostgreSQL 连接方式；独立于数据库资源的生命周期。
#[dill::component]
#[dill::interface(dyn DatabaseProvider)]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct PostgresPlugin;

#[async_trait::async_trait]
impl DatabaseProvider for PostgresPlugin {
    async fn connect(&self, config: &HostConfig) -> Result<PgPool> {
        PgPoolOptions::new()
            .max_connections(8)
            .connect(&config.database_url)
            .await
            .context("连接插件运行时 PostgreSQL 失败")
    }
}

impl Plugin<RuntimeAssembly> for PostgresPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.services.add::<PostgresPlugin>();
            Ok(())
        })
    }
}

/// 初始化数据库并持有关闭责任，具体连接实现通过 DatabaseProvider 注入。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct DatabasePlugin;

impl Plugin<RuntimeAssembly> for DatabasePlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.require_service::<dyn DatabaseProvider>();
            assembly.declare_resource::<PgPool>()?;
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    let pool = state
                        .service::<dyn DatabaseProvider>()?
                        .connect(&state.config)
                        .await?;
                    state.service::<Resource<PgPool>>()?.provide(Arc::new(pool))
                })
            });
            assembly.on_stop::<Self>(|state, _| {
                Box::pin(async move {
                    if let Some(pool) = state.service::<Resource<PgPool>>()?.initialized() {
                        pool.close().await;
                    }
                    Ok(())
                })
            });
            Ok(())
        })
    }
}

/// 平台持久化结构及开发会话清理依赖数据库能力。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct StorePlugin;

impl Plugin<RuntimeAssembly> for StorePlugin {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![TypeId::of::<DatabasePlugin>()]
    }
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.declare_resource::<PluginStore>()?;
            assembly
                .services
                .bind::<dyn super::super::plugin_catalog::PluginCatalog, PluginStore>();
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    let store = Arc::new(PluginStore::new(
                        state.service::<PgPool>()?.as_ref().clone(),
                    ));
                    store.migrate().await?;
                    if state.config.development.is_some() {
                        // 开发进程由本次 CLI 管理，旧地址不能在下次启动时复用。
                        sqlx::query(
                            "DELETE FROM tenant_plugin_bindings WHERE tenant_id='development'",
                        )
                        .execute(&store.pool)
                        .await?;
                    }
                    state.service::<Resource<PluginStore>>()?.provide(store)
                })
            });
            Ok(())
        })
    }
}
