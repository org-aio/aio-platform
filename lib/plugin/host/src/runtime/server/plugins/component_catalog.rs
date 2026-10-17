use std::{any::TypeId, sync::Arc};

use az_plugin_core::{Plugin, PluginFuture};

use super::{Resource, RuntimeAssembly, StorePlugin};

pub use super::super::component_catalog::*;

/// 默认整包目录使用既有平台表；替代目录只需绑定 ComponentCatalog，无须此插件。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct PostgresComponentCatalogPlugin;

impl Plugin<RuntimeAssembly> for PostgresComponentCatalogPlugin {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![TypeId::of::<StorePlugin>()]
    }

    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            if assembly.config.component_storage.is_none() {
                return Ok(());
            }
            assembly.declare_resource::<PostgresComponentCatalog>()?;
            assembly
                .services
                .bind::<dyn ComponentCatalog, PostgresComponentCatalog>();
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    let pool = state.service::<sqlx::PgPool>()?.as_ref().clone();
                    let catalog = PostgresComponentCatalog::open(pool).await?;
                    state
                        .service::<Resource<PostgresComponentCatalog>>()?
                        .provide(Arc::new(catalog))
                })
            });
            Ok(())
        })
    }
}
