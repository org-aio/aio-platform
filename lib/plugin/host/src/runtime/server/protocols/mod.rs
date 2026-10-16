//! 协议仅适配消息；身份、租户、能力授权和版本执行共用现有宿主服务。

mod access;
pub(super) mod grpc;
pub(super) mod mcp;
#[cfg(test)]
mod tests;
