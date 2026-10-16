//! 默认实现分别拥有 Wasm 执行槽和 Sidecar 资源，组件管理器只使用公共执行器契约。

mod sidecar;
mod wasm;

pub use sidecar::SidecarExecutor;
pub use wasm::WasmComponentExecutor;
