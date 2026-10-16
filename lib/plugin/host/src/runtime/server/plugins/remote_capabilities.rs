use std::{any::TypeId, sync::Arc, time::Duration};

use az_plugin_core::{Plugin, PluginFuture};

use super::super::remote::RemoteCapabilities;
use super::{CapabilityProvider, Resource, RuntimeAssembly, ScriptRuntimePlugin};

/// 可选的远端能力提供者；地址、契约和租户绑定由部署方文件提供，不开放任意 URL 代理。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct RemoteCapabilitiesPlugin;

impl Plugin<RuntimeAssembly> for RemoteCapabilitiesPlugin {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![TypeId::of::<ScriptRuntimePlugin>()]
    }

    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            if assembly.config.remote_capabilities.is_none() {
                return Ok(());
            }
            assembly.declare_resource::<RemoteCapabilities>()?;
            assembly
                .services
                .bind::<dyn CapabilityProvider, RemoteCapabilities>();
            assembly.on_initialize::<Self>(|state, tasks| {
                Box::pin(async move {
                    use anyhow::Context;
                    let path = state
                        .config
                        .remote_capabilities
                        .as_deref()
                        .context("远端能力配置缺失")?;
                    let capabilities = tokio::time::timeout(
                        Duration::from_secs(30),
                        RemoteCapabilities::open(path, tasks),
                    )
                    .await
                    .context("导入远端能力超时")??;
                    state
                        .service::<Resource<RemoteCapabilities>>()?
                        .provide(Arc::new(capabilities))
                })
            });
            assembly.on_stop::<Self>(|state, _| {
                Box::pin(async move {
                    if let Some(capabilities) = state
                        .service::<Resource<RemoteCapabilities>>()?
                        .initialized()
                    {
                        capabilities.shutdown();
                    }
                    Ok(())
                })
            });
            Ok(())
        })
    }
}
