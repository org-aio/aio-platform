//! 把部署方声明的外部工具映射为统一宿主能力；协议实现不参与授权决策。

mod bindings;
mod configuration;
#[cfg(feature = "remote-grpc")]
mod grpc;
#[cfg(feature = "remote-mcp")]
mod mcp;
#[cfg(feature = "remote-mcp")]
mod mcp_http;
#[cfg(feature = "remote-mcp")]
mod mcp_stdio;
mod provider;
#[cfg(test)]
mod tests;

pub(super) use provider::RemoteCapabilities;
