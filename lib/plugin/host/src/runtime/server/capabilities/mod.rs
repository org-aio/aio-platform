//! 原生、脚本和远端能力共用的目录、调用快照与授权执行链。

mod catalog;
#[cfg(test)]
mod catalog_tests;
mod controller;
mod invocation;
mod provider;

pub use catalog::RuntimeCapabilities;
pub(in crate::runtime::server) use controller::router;
pub use invocation::CapabilityInvocation;
pub(in crate::runtime::server) use invocation::CapabilityLifetime;
pub use provider::{CapabilityEffect, CapabilityProvider, CapabilityRequest, RuntimeCapability};
