#![forbid(unsafe_code)]

mod enhancement;
mod model;
pub mod process;

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
