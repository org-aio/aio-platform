#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

#[cfg(feature = "infrastructure")]
pub mod database;
#[cfg(feature = "infrastructure")]
pub mod http;
pub mod plugin;
#[cfg(feature = "infrastructure")]
mod record_validation;
#[cfg(feature = "infrastructure")]
mod records;

#[cfg(feature = "infrastructure")]
pub use database::*;
pub use plugin::*;
#[cfg(feature = "infrastructure")]
pub use record_validation::*;
#[cfg(feature = "infrastructure")]
pub use records::*;
