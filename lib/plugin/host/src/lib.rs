#![forbid(unsafe_code)]

pub mod configuration;
pub mod identity;
pub mod runtime;
mod startup;

#[cfg(any(feature = "web", feature = "desktop"))]
pub mod composition;
#[cfg(feature = "server")]
pub mod static_files;
#[cfg(any(feature = "web", feature = "desktop"))]
mod workspace;
#[cfg(any(feature = "web", feature = "desktop"))]
pub use workspace::Workspace;
#[cfg(any(feature = "web", feature = "desktop"))]
mod development;
#[cfg(any(feature = "web", feature = "desktop"))]
pub use development::DevelopmentStatus;

pub mod generated;
