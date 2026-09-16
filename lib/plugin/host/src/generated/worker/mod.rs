#[cfg(feature = "server")]
mod archive;
#[cfg(feature = "server")]
pub(crate) mod controller;
pub mod model;
#[cfg(any(feature = "web", feature = "desktop"))]
mod pairing;
#[cfg(feature = "server")]
mod service;
#[cfg(feature = "server")]
mod service_impl;
#[cfg(all(test, feature = "server"))]
mod tests;
#[cfg(feature = "server")]
mod util;
#[cfg(any(feature = "web", feature = "desktop"))]
pub(crate) mod view;
#[cfg(feature = "server")]
pub(crate) use service::WorkerService;
#[cfg(feature = "server")]
pub(crate) use service_impl::WorkerServiceImpl;

#[cfg(any(feature = "web", feature = "desktop"))]
mod task_form;
