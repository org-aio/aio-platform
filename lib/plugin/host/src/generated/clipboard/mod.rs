#[cfg(feature = "server")]
pub(crate) mod controller;
pub mod model;
#[cfg(feature = "server")]
mod service;
#[cfg(feature = "server")]
mod service_impl;
#[cfg(feature = "server")]
mod util;
#[cfg(feature = "server")]
pub(crate) use service::ClipboardService;
#[cfg(feature = "server")]
pub(crate) use service_impl::ClipboardServiceImpl;
#[cfg(all(test, feature = "server"))]
mod tests;
