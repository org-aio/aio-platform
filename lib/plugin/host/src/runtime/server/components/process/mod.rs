mod broker;
mod configuration;
#[cfg(all(test, feature = "postgres"))]
mod database_tests;
mod development;
mod instance;
mod management;
mod model;
mod resource_requests;
mod storage;
#[cfg(all(test, feature = "postgres"))]
mod storage_tests;
pub(in crate::runtime::server) mod supervision;
mod workers;

pub(in crate::runtime::server) use model::{Start, Stop};

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Weak},
};
use tokio::sync::Mutex;
use uuid::Uuid;

pub(super) struct Processes {
    components: Weak<super::Components>,
    root: PathBuf,
    supervisor: reqwest::Client,
    instances: Mutex<HashMap<(Uuid, String), Arc<model::Instance>>>,
    owned: Mutex<Vec<String>>,
    pub(super) pending: Mutex<HashMap<Uuid, Arc<model::Instance>>>,
}

mod enhancements;
mod http_egress;
mod identity;

#[cfg(all(test, feature = "postgres"))]
mod storage_probe_tests;
