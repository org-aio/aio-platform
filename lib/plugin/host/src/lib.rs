#![forbid(unsafe_code)]
#![cfg_attr(feature = "server", feature(unsize))]

#[cfg(feature = "server")]
pub mod authorization;
pub mod configuration;
pub mod identity;
#[cfg(feature = "server")]
pub mod metering;
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
