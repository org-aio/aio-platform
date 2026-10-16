//! 默认实现分别拥有 Wasm 执行槽和 Sidecar 资源，组件管理器只使用公共执行器契约。

#[cfg(feature = "sidecar")]
mod sidecar;
#[cfg(feature = "wasm")]
mod wasm;

#[cfg(feature = "sidecar")]
pub use sidecar::SidecarExecutor;
#[cfg(feature = "wasm")]
pub use wasm::WasmComponentExecutor;
