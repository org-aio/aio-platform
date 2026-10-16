//! 把部署方声明的外部工具映射为统一宿主能力；协议实现不参与授权决策。

mod bindings;
mod configuration;
mod grpc;
mod mcp;
mod mcp_http;
mod mcp_stdio;
mod provider;
#[cfg(test)]
mod tests;

pub(super) use provider::RemoteCapabilities;
