#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod model;
mod package;
mod rhai;
mod typescript;

pub use model::{Enhancement, Example, Language, Package, ScriptContext};
pub use package::PreparedPackage;

pub const MAX_PACKAGE_BYTES: usize = 256 * 1024;
pub const MAX_VALUE_BYTES: usize = 64 * 1024;
const MAX_SOURCE_BYTES: usize = 32 * 1024;
const EXECUTION_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(100);

#[cfg(test)]
mod tests;
