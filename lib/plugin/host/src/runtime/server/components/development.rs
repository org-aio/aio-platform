use super::{Components, execution::ActiveExecution, model::Description};
use anyhow::Result;
use az_plugin_bundle::VerifiedBundle;
use az_plugin_development::DevArtifact;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub(super) struct Instance {
    pub source: String,
    pub repository: String,
    #[cfg(feature = "sidecar")]
    pub backend_digest: String,
    pub generation: String,
    pub bundle: Arc<VerifiedBundle>,
    pub description: Description,
}

impl Components {
    pub(in crate::runtime::server) async fn prepare_development(
        &self,
        artifact: &DevArtifact,
        bundle: Arc<VerifiedBundle>,
    ) -> Result<az_plugin_development::DevLaunch> {
        self.executors
            .select(&bundle)?
            .prepare_development(artifact, bundle)
            .await
    }

    pub(in crate::runtime::server) async fn activate_development(
        &self,
        artifact: &DevArtifact,
        bundle: Arc<VerifiedBundle>,
        repository: &str,
    ) -> Result<()> {
        let source = Uuid::new_v5(&Uuid::NAMESPACE_URL, artifact.source.as_bytes());
        let executor = self.executors.select(&bundle)?;
        let description = executor
            .activate_development(artifact, bundle.clone())
            .await?;
        let instance = Instance {
            source: artifact.source.clone(),
            repository: repository.into(),
            #[cfg(feature = "sidecar")]
            backend_digest: artifact.backend_digest.clone(),
            generation: artifact.generation.to_string(),
            bundle: bundle.clone(),
            description,
        };
        let binding = ActiveExecution {
            digest: bundle.digest().into(),
            executor: executor.clone(),
        };
        // 描述和分发绑定在同一可见边界内更新，旧视图不能借用新执行器。
        let mut development = self.development.write().await;
        let mut active = self.active.write().await;
        let previous = active.insert((source, "development".into()), binding);
        development.insert(source, instance);
        drop(active);
        drop(development);
        if let Some(previous) = previous
            && previous.executor.as_ref().type_id() != executor.as_ref().type_id()
        {
            previous.executor.deactivate(source, "development").await?;
        }
        Ok(())
    }

    pub(in crate::runtime::server) async fn discard_development_candidate(&self) -> Result<()> {
        let mut errors = Vec::new();
        for executor in &self.executors.implementations {
            if let Err(error) = executor.discard_development_candidate().await {
                errors.push(format!("{error:#}"));
            }
        }
        anyhow::ensure!(errors.is_empty(), "清理开发候选失败: {}", errors.join("; "));
        Ok(())
    }
}
