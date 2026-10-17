#![forbid(unsafe_code)]

mod enhancement;
mod model;
pub mod process;
pub mod transport;
#[cfg(feature = "validation")]
mod value_schema;
#[cfg(feature = "validation")]
pub use value_schema::ValueSchema;
#[cfg(all(test, feature = "validation"))]
mod value_schema_tests;

/// 原生客户端与宿主共享的 gRPC 契约；Wasm 构建不启用此 feature。
#[cfg(feature = "grpc")]
pub mod grpc {
    tonic::include_proto!("aio.runtime.v1");
}

pub use enhancement::*;
pub use model::*;

pub const WIT: &str = include_str!("../wit/plugin.wit");
pub const ABI_VERSION: u32 = 2;

#[cfg(test)]
mod tests {
    #[test]
    fn contract_is_a_valid_versioned_world() -> anyhow::Result<()> {
        let mut resolve = wit_parser::Resolve::default();
        let package = resolve.push_str("plugin.wit", super::WIT)?;
        let world = resolve.select_world(&[package], Some("plugin"))?;
        assert_eq!(resolve.worlds[world].exports.len(), 4);
        Ok(())
    }
}
