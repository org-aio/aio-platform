//! 语言实现、脚本运行时与持久化增强服务分别装配，协议入口不依赖组件执行环境。

use std::{any::TypeId, sync::Arc};

use az_plugin_core::{Plugin, PluginFuture};
use az_plugin_enhancement::{RhaiExecutor, TypeScriptExecutor};
pub use az_plugin_enhancement::{ScriptExecutor, ScriptProgram, ScriptRuntime};
use sqlx::PgPool;

use super::super::enhancements::Enhancements;
use super::{DatabasePlugin, Resource, RuntimeAssembly};

/// 默认 Rhai 实现；应用可以省略该插件并注册自己的同语言执行器。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct RhaiPlugin;

impl Plugin<RuntimeAssembly> for RhaiPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.services.add::<RhaiExecutor>();
            Ok(())
        })
    }
}

/// 默认 TypeScript 实现；执行器选择与协议选择互相独立。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct TypeScriptPlugin;

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
            TypeId::of::<DatabasePlugin>(),
            TypeId::of::<ScriptRuntimePlugin>(),
        ]
    }

    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.declare_resource::<Enhancements>()?;
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    let enhancements = Enhancements::open(
                        state.service::<PgPool>()?.as_ref().clone(),
                        state.service::<ScriptRuntime>()?,
                    )
                    .await?;
                    state
                        .service::<Resource<Enhancements>>()?
                        .provide(Arc::new(enhancements))
                })
            });
            Ok(())
        })
    }
}
