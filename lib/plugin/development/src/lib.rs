#[cfg(not(target_arch = "wasm32"))]
mod artifact;
mod configuration;
#[cfg(not(target_arch = "wasm32"))]
pub use artifact::{artifact_digest, backend_digest};
#[cfg(not(target_arch = "wasm32"))]
mod fingerprint;
mod model;
mod resolution;
pub use resolution::{DependencyCandidate, matches_requirement, resolve_dependencies};

pub use configuration::{read, validate_relative};
#[cfg(not(target_arch = "wasm32"))]
pub use fingerprint::{fingerprint, source_identity};
pub use model::*;

pub const HOST_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
