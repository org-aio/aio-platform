use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Weak};
use tokio::task::JoinSet;
use uuid::Uuid;

use super::super::Components;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::runtime::server) struct Start {
    pub source: Uuid,
    pub tenant: String,
    pub revision: String,
}

impl Start {
    pub fn id(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(format!(
                "{}:{}:{}:{}",
                self.tenant.len(),
                self.tenant,
                self.source,
                self.revision
            ))
        )[..24]
            .into()
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::runtime::server) struct Stop {
    pub id: String,
}

pub(in crate::runtime::server::components) struct Instance {
    pub start: Start,
    pub bundle: Arc<az_plugin_bundle::VerifiedBundle>,
    pub token: String,
    pub client: reqwest::Client,
    pub resources: Arc<Resources>,
}

pub(super) struct Gateway {
    pub components: Weak<Components>,
    pub start: Start,
    pub token: String,
    pub endpoints: Vec<String>,
    pub http_endpoints: Vec<String>,
    pub worker_capabilities: Vec<String>,
    pub services: Vec<String>,
    pub client: reqwest::Client,
    pub quota: Arc<tokio::sync::Semaphore>,
}

pub(in crate::runtime::server::components) struct Resources {
    pub _jobs: JoinSet<()>,
    pub _socket_alias: Option<super::development::SocketAlias>,
}
