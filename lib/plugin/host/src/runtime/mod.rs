pub mod model;

#[cfg(any(feature = "web", feature = "desktop"))]
pub mod client;
#[cfg(any(feature = "web", feature = "desktop"))]
mod frontend;
#[cfg(any(feature = "web", feature = "desktop"))]
pub(crate) mod frontend_preload;
#[cfg(feature = "server")]
pub mod server;

pub use model::*;
