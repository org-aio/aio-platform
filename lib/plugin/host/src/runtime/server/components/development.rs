use super::{Components, model::Description};
use anyhow::{Context, Result};
use az_plugin_bundle::VerifiedBundle;
use az_plugin_development::DevArtifact;
use az_plugin_runtime::ComponentSlot;
use std::sync::Arc;
use uuid::Uuid;

pub(super) struct Instance {
    pub source: String,
    pub repository: String,
    pub backend_digest: String,
    pub generation: String,
    pub bundle: Arc<VerifiedBundle>,
    pub description: Description,
    pub slot: Option<Arc<ComponentSlot>>,
}

impl Components {
    pub(in crate::runtime::server) async fn prepare_development(
        &self,
        artifact: &DevArtifact,
        bundle: Arc<VerifiedBundle>,
    ) -> Result<az_plugin_development::DevLaunch> {
        self.processes.prepare_development(artifact, bundle).await
    }

    pub(in crate::runtime::server) async fn activate_development(
        &self,
        artifact: &DevArtifact,
        bundle: Arc<VerifiedBundle>,
        repository: &str,
    ) -> Result<()> {
        let source = Uuid::new_v5(&Uuid::NAMESPACE_URL, artifact.source.as_bytes());
        if bundle.manifest().plugin.runtime.process.is_some() {
            let description = self
                .processes
                .activate_development(artifact, bundle.clone())
                .await?;
            self.development.write().await.insert(
                source,
                Instance {
                    source: artifact.source.clone(),
                    repository: repository.into(),
                    backend_digest: artifact.backend_digest.clone(),
                    generation: artifact.generation.to_string(),
                    bundle,
                    description,
                    slot: None,
                },
            );
            return Ok(());
        }
        let previous = self
            .development
            .read()
            .await
            .get(&source)
            .and_then(|instance| instance.slot.clone());
        let slot = if let Some(slot) = previous {
            slot
        } else {
            Arc::new(ComponentSlot::new(
                source,
                "development".into(),
                semver::Version::parse(env!("CARGO_PKG_VERSION"))?,
            )?)
        };
        if !slot.refresh_frontend(bundle.clone()).await? {
            let resources = self
                .resources_verified(source, "development", &bundle)
                .await?;
            slot.replace(
                &self.engine,
                bundle.clone(),
                bundle.manifest().plugin.capabilities.clone(),
                resources,
            )
            .await?;
        }
        let description = slot
            .snapshot()
            .await
            .context("开发实例未激活")?
            .description
            .into();
        self.development.write().await.insert(
            source,
            Instance {
                source: artifact.source.clone(),
                repository: repository.into(),
                backend_digest: artifact.backend_digest.clone(),
                generation: artifact.generation.to_string(),
                bundle,
                description,
                slot: Some(slot),
            },
        );
        Ok(())
    }
}

impl super::Components {
    pub(in crate::runtime::server) async fn discard_development_candidate(&self) {
        self.processes.pending.lock().await.clear();
    }
}
