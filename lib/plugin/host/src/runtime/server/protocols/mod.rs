//! 协议仅适配消息；身份、租户、能力授权和版本执行共用现有宿主服务。

#[cfg(any(feature = "mcp", feature = "grpc"))]
mod access;
#[cfg(feature = "grpc")]
pub(super) mod grpc;
#[cfg(feature = "grpc")]
mod grpc_capabilities;
#[cfg(feature = "grpc")]
mod grpc_enhancements;
#[cfg(feature = "mcp")]
pub(super) mod mcp;
#[cfg(feature = "mcp")]
mod mcp_catalog;
#[cfg(feature = "mcp")]
mod mcp_schema;
#[cfg(all(
    test,
    feature = "mcp",
    feature = "grpc",
    feature = "rhai",
    feature = "typescript"
))]
#[cfg(feature = "postgres")]
mod tests;
