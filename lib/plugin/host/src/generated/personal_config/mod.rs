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
pub(crate) use service::PersonalConfigService;
#[cfg(feature = "server")]
pub(crate) use service_impl::PersonalConfigServiceImpl;
#[cfg(all(test, feature = "server"))]
mod tests;
