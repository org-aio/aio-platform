#[cfg(feature = "server")]
mod archive;
#[cfg(feature = "server")]
pub(crate) mod controller;
pub mod model;
#[cfg(any(feature = "web", feature = "desktop"))]
mod pairing;
#[cfg(feature = "server")]
mod service;
#[cfg(feature = "postgres")]
mod service_impl;
#[cfg(all(test, feature = "postgres"))]
mod storage_tests;
#[cfg(all(test, feature = "postgres"))]
mod tests;
#[cfg(feature = "postgres")]
mod util;
#[cfg(any(feature = "web", feature = "desktop"))]
pub(crate) mod view;
#[cfg(feature = "server")]
pub use service::{WorkerService, WorkerServiceUnavailable};
#[cfg(feature = "postgres")]
pub(crate) use service_impl::WorkerServiceImpl;
