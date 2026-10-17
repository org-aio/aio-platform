//! 语言实现、脚本运行时与持久化增强服务分别装配，协议入口不依赖组件执行环境。

use std::{any::TypeId, sync::Arc};

use az_plugin_core::{Plugin, PluginFuture};
#[cfg(feature = "rhai")]
use az_plugin_enhancement::RhaiExecutor;
#[cfg(feature = "typescript")]
use az_plugin_enhancement::TypeScriptExecutor;
pub use az_plugin_enhancement::{ScriptExecutor, ScriptInvocation, ScriptProgram, ScriptRuntime};

use super::super::enhancement_catalog::EnhancementCatalog;
use super::super::enhancements::CapabilityEnvironment;
use super::super::enhancements::Enhancements;
use super::{CapabilityServicePlugin, Resource, RuntimeAssembly};

/// 默认 Rhai 实现；应用可以省略该插件并注册自己的同语言执行器。
#[cfg(feature = "rhai")]
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct RhaiPlugin;

#[cfg(feature = "rhai")]
impl Plugin<RuntimeAssembly> for RhaiPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.services.add::<RhaiExecutor>();
            Ok(())
        })
    }
}

/// 默认 TypeScript 实现；执行器选择与协议选择互相独立。
#[cfg(feature = "typescript")]
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct TypeScriptPlugin;

#[cfg(feature = "typescript")]
impl Plugin<RuntimeAssembly> for TypeScriptPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.services.add::<TypeScriptExecutor>();
            Ok(())
        })
    }
}

/// 在同一 Dill 容器汇集脚本执行器；此插件本身不需要数据库、密钥或文件系统。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct ScriptRuntimePlugin;

impl Plugin<RuntimeAssembly> for ScriptRuntimePlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.services.add::<ScriptRuntime>();
            assembly.require_service::<ScriptRuntime>();
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move { state.service::<ScriptRuntime>()?.validate() })
            });
            Ok(())
        })
    }
}

/// 统一持有增强版本和执行服务；WIT、Sidecar、HTTP、MCP、gRPC 复用同一实例。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct EnhancementServicePlugin;

impl Plugin<RuntimeAssembly> for EnhancementServicePlugin {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![
            TypeId::of::<ScriptRuntimePlugin>(),
            TypeId::of::<CapabilityServicePlugin>(),
        ]
    }

    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.require_service::<dyn crate::identity::IdentityProvider>();
            assembly.require_service::<dyn EnhancementCatalog>();
            assembly.observe_catalog();
            assembly.declare_resource::<Enhancements>()?;
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    let enhancements = Enhancements::new(
                        state.service::<dyn EnhancementCatalog>()?,
                        state.service::<ScriptRuntime>()?,
                        Some(CapabilityEnvironment {
                            services: Arc::downgrade(&state.extensions),
                            identity: state.service::<dyn crate::identity::IdentityProvider>()?,
                        }),
                    );
                    state
                        .service::<Resource<Enhancements>>()?
                        .provide(Arc::new(enhancements))
                })
            });
            Ok(())
        })
    }
}
