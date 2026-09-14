//! 产品集成测试使用的隔离夹具，不随正式宿主启用。
use super::RuntimeState;
use anyhow::Result;
pub use az_plugin_package::PluginPackage;
use std::sync::Arc;

pub use super::repository::{DiscoveredPlugin, RepositoryInstaller};
pub use super::store::PluginStore;
pub use super::supervisor::ProcessInstance;

impl RuntimeState {
    pub async fn cache_test_package(&self, package: &PluginPackage) -> Result<()> {
        self.store.save_package(package).await?;
        self.repository.stage_publish(package).await?;
        Ok(())
    }
    pub async fn has_test_package(&self, revision: &str) -> Result<bool> {
        Ok(self.store.package_archive(revision).await?.is_some())
    }
    pub fn database(&self) -> &sqlx::PgPool {
        &self.store.pool
    }
    pub async fn hold_publications(&self) -> Result<tokio::sync::OwnedSemaphorePermit> {
        Ok(self.publication_slots.clone().acquire_many_owned(2).await?)
    }
}

pub fn session_context(session: &crate::identity::SessionContext) -> String {
    super::request_context::session_context(session)
}

pub fn tenant_context(session: &crate::identity::SessionContext) -> Result<String> {
    super::request_context::tenant_context(session)
}

pub async fn prepare_frontend(
    state: &RuntimeState,
    revision: &str,
    entry: &str,
) -> Result<Arc<dyn std::any::Any + Send + Sync>> {
    Ok(super::frontend_package::prepare(state, revision, entry).await?)
}

pub async fn exercise_rollouts(
    state: &RuntimeState,
    base: &str,
    manifest: &str,
    pages: &str,
) -> Result<()> {
    super::delivery::exercise_rollouts(state, base, manifest, pages).await
}
