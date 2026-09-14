use anyhow::Result;

use super::{
    repository::DiscoveredPlugin,
    store::{PluginStore, record_event, runtime_name, start_instance, stop_instances},
    supervisor::ProcessInstance,
};
use crate::runtime::MarketplaceEntry;

impl PluginStore {
    pub async fn activate(
        &self,
        tenant_id: &str,
        plugin: DiscoveredPlugin,
        instance: Option<&ProcessInstance>,
        publication: Option<&MarketplaceEntry>,
    ) -> Result<()> {
        self.save_activation(tenant_id, plugin, instance, publication, true)
            .await
    }

    pub(super) async fn publish_only(
        &self,
        tenant_id: &str,
        plugin: DiscoveredPlugin,
        publication: &MarketplaceEntry,
    ) -> Result<()> {
        self.save_activation(tenant_id, plugin, None, Some(publication), false)
            .await
    }

    async fn save_activation(
        &self,
        tenant_id: &str,
        plugin: DiscoveredPlugin,
        instance: Option<&ProcessInstance>,
        publication: Option<&MarketplaceEntry>,
        bind: bool,
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        if publication.is_some() {
            super::delivery::ensure_current(&mut transaction, &plugin.git, &plugin.revision)
                .await?;
            super::publisher_store::ensure_current_publication(
                &mut transaction,
                tenant_id,
                &plugin.git,
                &plugin.revision,
            )
            .await?;
        }
        sqlx::query(
            "INSERT INTO plugin_sources (id, git) VALUES ($1, $2) ON CONFLICT (git) DO UPDATE SET git = EXCLUDED.git",
        )
        .bind(&plugin.source_id)
        .bind(&plugin.git)
        .execute(&mut *transaction)
        .await?;
        let source_id =
            sqlx::query_scalar::<_, String>("SELECT id FROM plugin_sources WHERE git = $1")
                .bind(&plugin.git)
                .fetch_one(&mut *transaction)
                .await?;
        let requested_revision_id = format!("{source_id}:{}", plugin.revision);
        let revision_id = sqlx::query_scalar::<_, String>(
            "INSERT INTO plugin_revisions (id, source_id, revision, runtime, manifest, pages) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (source_id, revision) DO UPDATE SET manifest = EXCLUDED.manifest, pages = EXCLUDED.pages RETURNING id",
        )
        .bind(&requested_revision_id)
        .bind(&source_id)
        .bind(&plugin.revision)
        .bind(runtime_name(plugin.runtime))
        .bind(&plugin.manifest)
        .bind(serde_json::to_value(&plugin.pages)?)
        .fetch_one(&mut *transaction)
        .await?;
        if bind {
            sqlx::query(
            "INSERT INTO tenant_plugin_bindings (tenant_id, source_id, revision_id, enabled) VALUES ($1, $2, $3, TRUE) ON CONFLICT (tenant_id, source_id) DO UPDATE SET revision_id = EXCLUDED.revision_id, enabled = TRUE, updated_at = now()",
        )
        .bind(tenant_id)
        .bind(&source_id)
        .bind(&revision_id)
        .execute(&mut *transaction)
        .await?;
            super::delivery::remember_installation(&mut transaction, tenant_id, &source_id).await?;
            stop_instances(&mut transaction, tenant_id, &source_id).await?;
            start_instance(&mut transaction, tenant_id, &revision_id, instance).await?;
            record_event(
                &mut transaction,
                tenant_id,
                &source_id,
                Some(&revision_id),
                "activate",
                "健康检查通过并原子激活",
            )
            .await?;
        }
        if let Some(publication) = publication {
            super::marketplace_store::upsert_published_marketplace_entry(
                &mut transaction,
                publication,
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}
