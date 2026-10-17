use std::sync::Arc;

use az_plugin_core::{Plugin, PluginFuture};

use super::super::capabilities::CapabilityLifetime;
pub use super::super::capabilities::{
    CapabilityEffect, CapabilityInvocation, CapabilityProvider, CapabilityRequest,
    RuntimeCapabilities, RuntimeCapability,
};
use super::{Resource, RuntimeAssembly};

/// 汇集原生和远端能力；不依赖脚本执行器、增强版本存储或组件环境。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct CapabilityServicePlugin;

impl Plugin<RuntimeAssembly> for CapabilityServicePlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.services.add::<RuntimeCapabilities>();
            assembly.require_service::<RuntimeCapabilities>();
            // 替换身份提供者时一并失效旧调用链；仅使用目录的组合可以省略身份服务。
            assembly.optional_service::<dyn crate::identity::IdentityProvider>();
            assembly.declare_resource::<CapabilityLifetime>()?;
            assembly.on_initialize::<Self>(|state, tasks| {
                Box::pin(async move {
                    state
                        .service::<Resource<CapabilityLifetime>>()?
                        .provide(Arc::new(CapabilityLifetime(tasks)))
                })
            });
            // 提供者可依赖异步资源；全部初始化完成后、启动副作用前校验目录。
            assembly.requirements.push(|services, resolve| {
                if resolve {
                    services.get_one::<RuntimeCapabilities>()?.validate()?;
                }
                Ok(())
            });
            Ok(())
        })
    }
}
