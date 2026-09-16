mod broker;
mod configuration;
mod development;
mod instance;
mod model;
pub(in crate::runtime::server) mod supervision;

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
    pub(super) pending: Mutex<HashMap<Uuid, Arc<model::Instance>>>,
}

mod http_egress;
