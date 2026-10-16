use std::{any::TypeId, sync::Arc};

use az_plugin_core::{Plugin, PluginFuture};

use super::super::enhancements::{Enhancements, EvolutionCapabilities};
use super::{CapabilityProvider, EnhancementServicePlugin, Resource, RuntimeAssembly};

/// 可省略的演进控制插件；不内置模型客户端，也不自行生成或定时激活代码。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct EvolutionPlugin;

impl Plugin<RuntimeAssembly> for EvolutionPlugin {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![TypeId::of::<EnhancementServicePlugin>()]
    }

    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.declare_resource::<EvolutionCapabilities>()?;
            assembly
                .services
                .bind::<dyn CapabilityProvider, EvolutionCapabilities>();
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    let capabilities = EvolutionCapabilities {
                        enhancements: state.service::<Enhancements>()?,
                        identity: state.identity.clone(),
                    };
                    state
                        .service::<Resource<EvolutionCapabilities>>()?
                        .provide(Arc::new(capabilities))
                })
            });
            Ok(())
        })
    }
}
