use anyhow::Result;
use az_plugin_development::PublishedRelease;

use super::{PluginCatalogQueries, PluginStore};
use crate::runtime::PageDefinition;

#[async_trait::async_trait]
impl PluginCatalogQueries for PluginStore {
    async fn source_git(&self, source: &str) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT git FROM plugin_sources WHERE id=$1")
                .bind(source)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    async fn upgrade_eligible(&self, tenant: &str, source: &str, revision: &str) -> Result<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tenant_plugin_bindings b JOIN plugin_sources s ON s.id=b.source_id JOIN marketplace_entries m ON m.git=s.git AND m.source='aio://published' LEFT JOIN delivery_installations i ON i.tenant_id=b.tenant_id AND i.source_id=b.source_id WHERE b.tenant_id=$1 AND b.source_id=$2 AND b.enabled AND m.rev=$3 AND i.excluded_revision IS DISTINCT FROM $3)")
            .bind(tenant).bind(source).bind(revision).fetch_one(&self.pool).await?)
    }

    async fn publication_only(&self, tenant: &str, source: &str, revision: &str) -> Result<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM delivery_jobs WHERE package_revision=$3) AND EXISTS(SELECT 1 FROM delivery_installations WHERE tenant_id=$1 AND source_id=$2) AND NOT EXISTS(SELECT 1 FROM tenant_plugin_bindings WHERE tenant_id=$1 AND source_id=$2 AND enabled)")
            .bind(tenant).bind(source).bind(revision).fetch_one(&self.pool).await?)
    }

    async fn source_exists(&self, git: &str) -> Result<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM marketplace_entries WHERE git=$1) OR EXISTS(SELECT 1 FROM plugin_sources WHERE git=$1)")
            .bind(git).fetch_one(&self.pool).await?)
    }

    async fn mark_removed(&self, git: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO marketplace_plugin_removals(git) VALUES($1) ON CONFLICT(git) DO NOTHING",
        )
        .bind(git)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn removed_sources(&self) -> Result<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT git FROM marketplace_plugin_removals")
                .fetch_all(&self.pool)
                .await?,
        )
    }

    async fn is_removed(&self, git: &str) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM marketplace_plugin_removals WHERE git=$1)",
        )
        .bind(git)
        .fetch_one(&self.pool)
        .await?)
    }

    async fn set_menu_hidden(&self, tenant: &str, source: &str, hidden: bool) -> Result<()> {
        let sql = if hidden {
            "INSERT INTO tenant_hidden_plugin_menus(tenant_id,source_id) VALUES($1,$2) ON CONFLICT DO NOTHING"
        } else {
            "DELETE FROM tenant_hidden_plugin_menus WHERE tenant_id=$1 AND source_id=$2"
        };
        sqlx::query(sql)
            .bind(tenant)
            .bind(source)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn hidden_sources(&self, tenant: &str) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar("SELECT source_id FROM tenant_hidden_plugin_menus WHERE tenant_id=$1 ORDER BY source_id")
            .bind(tenant).fetch_all(&self.pool).await?)
    }

    async fn hidden_page_ids(&self, tenant: &str, sources: &[String]) -> Result<Vec<String>> {
        let rows = sqlx::query_scalar::<_, serde_json::Value>("SELECT r.pages FROM tenant_plugin_bindings b JOIN plugin_revisions r ON r.id=b.revision_id WHERE b.tenant_id=$1 AND b.enabled AND b.source_id=ANY($2)")
            .bind(tenant).bind(sources).fetch_all(&self.pool).await?;
        let mut ids = Vec::new();
        for row in rows {
            ids.extend(
                serde_json::from_value::<Vec<PageDefinition>>(row)?
                    .into_iter()
                    .map(|page| page.id),
            );
        }
        Ok(ids)
    }

    async fn releases(&self, git: &str) -> Result<Vec<PublishedRelease>> {
        let rows = sqlx::query_as::<_, (String, String, String, String)>("SELECT p.version,p.source_revision,p.revision,p.manifest_toml FROM plugin_packages p WHERE p.git=$1 AND p.source_revision IS NOT NULL AND EXISTS(SELECT 1 FROM plugin_revisions r JOIN plugin_sources s ON s.id=r.source_id WHERE r.revision=p.revision AND s.git=p.git) ORDER BY p.created_at DESC LIMIT 100")
            .bind(git).fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|(version, source_sha, digest, manifest)| PublishedRelease {
                git: git.to_owned(),
                version,
                source_sha,
                digest,
                manifest,
                abi: 1,
            })
            .collect())
    }
}
