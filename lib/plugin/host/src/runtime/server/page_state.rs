use std::collections::HashMap;

use anyhow::{Context as _, Result, ensure};
use az_plugin_manifest::{PageBody, PageDefinition};
use sqlx::{PgPool, Postgres, Row, Transaction};

use super::store::PluginStore;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS plugin_page_states (
    tenant_id TEXT NOT NULL,
    revision_id TEXT NOT NULL REFERENCES plugin_revisions(id) ON DELETE CASCADE,
    page_id TEXT NOT NULL,
    body JSONB NOT NULL,
    generation BIGINT NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY(tenant_id, revision_id, page_id)
);
"#;

pub(super) async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql(SCHEMA)
        .execute(pool)
        .await
        .context("创建插件页面状态表失败")?;
    Ok(())
}

pub(super) async fn delete_source_states(
    transaction: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    source_id: &str,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM plugin_page_states states USING plugin_revisions revisions WHERE states.tenant_id = $1 AND states.revision_id = revisions.id AND revisions.source_id = $2",
    )
    .bind(tenant_id)
    .bind(source_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

impl PluginStore {
    pub(super) async fn overlay_page_states(
        &self,
        tenant_id: &str,
        revision_id: &str,
        pages: &mut [PageDefinition],
    ) -> Result<HashMap<String, i64>> {
        let rows = sqlx::query(
            "SELECT page_id, body, generation FROM plugin_page_states WHERE tenant_id = $1 AND revision_id = $2",
        )
        .bind(tenant_id)
        .bind(revision_id)
        .fetch_all(&self.pool)
        .await?;
        let mut pages_by_id = pages
            .iter_mut()
            .map(|page| (page.id.clone(), page))
            .collect::<HashMap<_, _>>();
        let mut generations = HashMap::new();
        for row in rows {
            let page_id: String = row.try_get("page_id")?;
            let Some(page) = pages_by_id.get_mut(&page_id) else {
                continue;
            };
            let body = serde_json::from_value::<PageBody>(row.try_get("body")?)
                .with_context(|| format!("解析页面状态失败: {page_id}"))?;
            page.body = body;
            generations.insert(page_id, row.try_get("generation")?);
        }
        Ok(generations)
    }

    pub async fn save_page_state(
        &self,
        tenant_id: &str,
        source_id: &str,
        revision_id: &str,
        page_id: &str,
        expected_generation: Option<i64>,
        body: &PageBody,
    ) -> Result<()> {
        let body = serde_json::to_value(body)?;
        let result = if let Some(generation) = expected_generation {
            sqlx::query(
                "UPDATE plugin_page_states states SET body = $6, generation = states.generation + 1, updated_at = now() WHERE states.tenant_id = $1 AND states.revision_id = $3 AND states.page_id = $4 AND states.generation = $5 AND EXISTS (SELECT 1 FROM tenant_plugin_bindings bindings WHERE bindings.tenant_id = $1 AND bindings.source_id = $2 AND bindings.revision_id = $3 AND bindings.enabled = TRUE)",
            )
            .bind(tenant_id)
            .bind(source_id)
            .bind(revision_id)
            .bind(page_id)
            .bind(generation)
            .bind(body)
            .execute(&self.pool)
            .await?
        } else {
            sqlx::query(
                "INSERT INTO plugin_page_states (tenant_id, revision_id, page_id, body) SELECT $1, $3, $4, $5 WHERE EXISTS (SELECT 1 FROM tenant_plugin_bindings bindings WHERE bindings.tenant_id = $1 AND bindings.source_id = $2 AND bindings.revision_id = $3 AND bindings.enabled = TRUE) ON CONFLICT (tenant_id, revision_id, page_id) DO NOTHING",
            )
            .bind(tenant_id)
            .bind(source_id)
            .bind(revision_id)
            .bind(page_id)
            .bind(body)
            .execute(&self.pool)
            .await?
        };
        ensure!(
            result.rows_affected() == 1,
            "页面状态已变化或插件版本已切换，请重试"
        );
        Ok(())
    }
}
