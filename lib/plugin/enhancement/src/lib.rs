#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod execution;
mod model;
mod package;
#[cfg(feature = "rhai")]
mod rhai;
#[cfg(feature = "typescript")]
mod typescript;

pub use execution::{ScriptExecutor, ScriptProgram, ScriptRuntime, default_runtime};
pub use model::{Enhancement, Example, Language, Package, ScriptContext};
pub use package::PreparedPackage;
#[cfg(feature = "rhai")]
pub use rhai::RhaiExecutor;
#[cfg(feature = "typescript")]
pub use typescript::TypeScriptExecutor;

pub const MAX_PACKAGE_BYTES: usize = 256 * 1024;
pub const MAX_VALUE_BYTES: usize = 64 * 1024;
const MAX_SOURCE_BYTES: usize = 32 * 1024;
#[cfg(any(feature = "rhai", feature = "typescript"))]
const EXECUTION_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(100);

#[cfg(test)]
mod execution_tests;
#[cfg(all(test, feature = "rhai", feature = "typescript"))]
mod tests;
