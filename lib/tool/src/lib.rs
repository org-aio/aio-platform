mod link;
mod model;
pub mod registration;
mod validation;

pub use link::InstallLink;
pub use model::{CommandSpec, InstallationPlan, Requirement, ToolManifest};

pub const OFFICIAL_ORIGIN: &str = "https://aio.addzero.site";
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

#[cfg(feature = "native")]
pub mod install;
#[cfg(feature = "native")]
pub mod protocol;

#[cfg(test)]
mod tests;
