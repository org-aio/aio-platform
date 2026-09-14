mod model;
pub(crate) use model::{ApplicationSnapshot, LoadedApplication};

#[cfg(any(feature = "web", feature = "desktop"))]
mod client;
#[cfg(any(feature = "web", feature = "desktop"))]
pub(crate) use client::load;
