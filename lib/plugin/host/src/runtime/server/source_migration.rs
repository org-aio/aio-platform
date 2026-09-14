use anyhow::Result;
use sqlx::Row as _;

use super::{repository::published_source_id, store::PluginStore};

impl PluginStore {
    pub(super) async fn migrate_published_source_ids(&self) -> Result<()> {
        let rows = sqlx::query(
            "SELECT id AS source_id, git FROM plugin_sources WHERE id LIKE 'publish-%' UNION SELECT source_id, git FROM plugin_publish_jobs WHERE source_id LIKE 'publish-%' ORDER BY source_id",
        )
        .fetch_all(&self.pool)
        .await?;
        for row in rows {
            let previous: String = row.try_get("source_id")?;
            let git: String = row.try_get("git")?;
            let replacement = published_source_id(&git);
            let mut transaction = self.pool.begin().await?;
            sqlx::query("UPDATE plugin_lifecycle_events SET source_id = $2 WHERE source_id = $1")
                .bind(&previous)
                .bind(&replacement)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("UPDATE plugin_publish_jobs SET source_id = $2 WHERE source_id = $1")
                .bind(&previous)
                .bind(&replacement)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("UPDATE plugin_sources SET id = $2 WHERE id = $1")
                .bind(&previous)
                .bind(&replacement)
                .execute(&mut *transaction)
                .await?;
            transaction.commit().await?;
        }
        Ok(())
    }
}
