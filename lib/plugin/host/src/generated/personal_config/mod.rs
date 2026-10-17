#[cfg(feature = "server")]
pub(crate) mod controller;
pub mod model;
#[cfg(feature = "server")]
mod service;
#[cfg(feature = "postgres")]
mod service_impl;
#[cfg(feature = "postgres")]
mod util;
#[cfg(feature = "server")]
pub use service::PersonalConfigService;
#[cfg(feature = "postgres")]
pub(crate) use service_impl::PersonalConfigServiceImpl;
#[cfg(all(test, feature = "postgres"))]
mod tests;
