#[cfg(test)]
mod access_tests;
mod catalog;
mod controller;
mod delivery;
#[cfg(test)]
mod delivery_tests;
mod development;
#[cfg(all(test, feature = "wasm", feature = "sidecar"))]
mod development_tests;
mod dispatch;
pub(in crate::runtime::server) mod execution;
pub(in crate::runtime::server) mod executors;
mod frontend;
mod installation;
mod model;
mod permissions;
#[cfg(feature = "sidecar")]
pub(in crate::runtime::server) mod process;
#[cfg(all(test, feature = "sidecar"))]
mod process_publication_tests;
mod resources;
mod services;
mod shutdown;
mod store;
#[cfg(test)]
mod tests;
mod worker_services;

use az_plugin_runtime::{DatabaseProvisioner, Keyring};
use sqlx::PgPool;
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use execution::{ActiveExecution, ComponentExecutors};

pub(super) use controller::router;
pub(super) use frontend::device_view_grant;
pub(super) use frontend::mount;
pub(super) use resources::load_keyring;

pub(super) struct Components {
    #[cfg(feature = "sidecar")]
    pub(super) enhancements: Arc<super::enhancements::Enhancements>,
    pub pool: PgPool,
    development: RwLock<std::collections::BTreeMap<Uuid, development::Instance>>,
    provisioner: DatabaseProvisioner,
    keyring: Arc<Keyring>,
    objects: PathBuf,
    services: Arc<services::Services>,
    #[cfg(feature = "sidecar")]
    identity: Arc<dyn crate::identity::IdentityProvider>,
    runtime_services: dill::CatalogWeakRef,
    executors: Arc<ComponentExecutors>,
    active: RwLock<HashMap<(Uuid, String), ActiveExecution>>,
    pub mutations: Mutex<()>,
}

#[cfg(test)]
mod settings_tests;
