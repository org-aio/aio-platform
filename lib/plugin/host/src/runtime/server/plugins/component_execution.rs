//! 整包执行器由应用显式选择；组件资源插件负责统一初始化、恢复和关闭。

use super::RuntimeAssembly;
use az_plugin_core::{Plugin, PluginFuture};

pub use super::super::components::execution::{
    ComponentDescription, ComponentExecutor, ComponentExecutors, ComponentPage, ExecutionContext,
    Request as ComponentRequest, Response as ComponentResponse,
};
#[cfg(feature = "sidecar")]
pub use super::super::components::executors::SidecarExecutor;
#[cfg(feature = "wasm")]
pub use super::super::components::executors::WasmComponentExecutor;

/// 选择默认 Wasm Component 实现；省略后不会创建对应执行引擎。
#[cfg(feature = "wasm")]
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct WasmComponentPlugin;

#[cfg(feature = "wasm")]
impl Plugin<RuntimeAssembly> for WasmComponentPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_value(WasmComponentExecutor::default())
                .bind::<dyn ComponentExecutor, WasmComponentExecutor>();
            Ok(())
        })
    }
}

/// 选择默认 Sidecar 实现；省略后不会初始化监督器客户端或进程资源。
#[cfg(feature = "sidecar")]
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct SidecarPlugin;

#[cfg(feature = "sidecar")]
impl Plugin<RuntimeAssembly> for SidecarPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_value(SidecarExecutor::default())
                .bind::<dyn ComponentExecutor, SidecarExecutor>();
            Ok(())
        })
    }
}

/// 汇集已选择的实现；契约本身不创建数据库、引擎、缓存或外部进程。
#[dill::component]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct ComponentExecutionPlugin;

impl Plugin<RuntimeAssembly> for ComponentExecutionPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.services.add::<ComponentExecutors>();
            assembly.require_service::<ComponentExecutors>();
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move { state.service::<ComponentExecutors>()?.validate() })
            });
            Ok(())
        })
    }
}
