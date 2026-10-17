use std::{any::TypeId, sync::Arc};

use az_plugin_core::{Plugin, PluginFuture};

use super::super::enhancement_catalog::{EnhancementCatalog, PostgresEnhancementCatalog};
use super::{DatabasePlugin, Resource, RuntimeAssembly};

/// 默认增强目录；应用可绑定其他 EnhancementCatalog 并省略此插件和数据库。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct PostgresEnhancementCatalogPlugin;

impl Plugin<RuntimeAssembly> for PostgresEnhancementCatalogPlugin {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![TypeId::of::<DatabasePlugin>()]
    }

    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.require_service::<sqlx::PgPool>();
            assembly.declare_resource::<PostgresEnhancementCatalog>()?;
            assembly
                .services
                .bind::<dyn EnhancementCatalog, PostgresEnhancementCatalog>();
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    let pool = state.service::<sqlx::PgPool>()?.as_ref().clone();
                    let catalog = PostgresEnhancementCatalog::open(pool).await?;
                    state
                        .service::<Resource<PostgresEnhancementCatalog>>()?
                        .provide(Arc::new(catalog))
                })
            });
            Ok(())
        })
    }
}
