use az_plugin_development::DevStatus;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{Mutex, RwLock};

use super::super::frontend_package::FrontendPackage;

#[derive(Default)]
pub(in crate::runtime::server) struct DevelopmentState {
    pub events: tokio::sync::watch::Sender<Option<DevStatus>>,
    pub mutation: Mutex<()>,
    pub artifacts: RwLock<BTreeMap<String, Arc<FrontendPackage>>>,
    pub versions: RwLock<BTreeMap<String, (u64, String)>>,
    pub status: RwLock<Option<DevStatus>>,
}

impl DevelopmentState {
    pub async fn frontend(&self, revision: &str) -> Option<Arc<FrontendPackage>> {
        self.artifacts.read().await.get(revision).cloned()
    }
}
