mod controller;
mod service;

pub(super) use controller::router;
#[cfg(feature = "postgres")]
pub(super) use service::migrate;
pub(in crate::runtime::server) use service::validate_navigation_label;
pub use service::{NavigationLabelKind, NavigationLabelOverride};
pub(super) use service::{apply, enrich_entries};
