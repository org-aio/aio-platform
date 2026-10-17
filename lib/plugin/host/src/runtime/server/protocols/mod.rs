//! 协议仅适配消息；身份、租户、能力授权和版本执行共用现有宿主服务。

#[cfg(any(feature = "mcp", feature = "grpc"))]
mod access;
#[cfg(feature = "grpc")]
pub(super) mod grpc;
#[cfg(feature = "mcp")]
pub(super) mod mcp;
#[cfg(all(
    test,
    feature = "mcp",
    feature = "grpc",
    feature = "rhai",
    feature = "typescript"
))]
mod tests;
