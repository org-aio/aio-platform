use anyhow::{Context as _, Result};
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgRow};

use super::store::{PluginStore, parse_runtime, runtime_name};
use crate::runtime::MarketplaceEntry;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS marketplace_entries (
    source TEXT NOT NULL,
    git TEXT NOT NULL,
    rev TEXT NOT NULL,
    title TEXT NOT NULL,
    summary TEXT NOT NULL,
    license TEXT NOT NULL,
    tags JSONB NOT NULL,
    runtime TEXT,
    capabilities JSONB NOT NULL,
    synced_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY(source, git)
);
CREATE INDEX IF NOT EXISTS marketplace_entries_source_title_idx ON marketplace_entries(source, title);
"#;

pub(super) const PUBLISHED_REGISTRY_SOURCE: &str = "aio://published";

pub(super) async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql(SCHEMA)
        .execute(pool)
        .await
        .context("创建市场缓存表失败")?;
    super::official_market::migrate(pool).await?;
    Ok(())
}

pub(super) async fn upsert_published_marketplace_entry(
    transaction: &mut Transaction<'_, Postgres>,
    entry: &MarketplaceEntry,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO marketplace_entries (source, git, rev, title, summary, license, tags, runtime, capabilities, synced_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now()) ON CONFLICT (source, git) DO UPDATE SET rev = EXCLUDED.rev, title = EXCLUDED.title, summary = EXCLUDED.summary, license = EXCLUDED.license, tags = EXCLUDED.tags, runtime = EXCLUDED.runtime, capabilities = EXCLUDED.capabilities, synced_at = now()",
    )
    .bind(PUBLISHED_REGISTRY_SOURCE)
    .bind(&entry.git)
    .bind(&entry.rev)
    .bind(&entry.title)
    .bind(&entry.summary)
    .bind(&entry.license)
    .bind(serde_json::to_value(&entry.tags)?)
    .bind(entry.runtime.map(runtime_name))
    .bind(serde_json::to_value(&entry.capabilities)?)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

impl PluginStore {
    pub async fn published_marketplace_entry(
        &self,
        git: &str,
        requested_revision: Option<&str>,
    ) -> Result<Option<MarketplaceEntry>> {
        if let Some(revision) =
            requested_revision.filter(|revision| super::repository::is_package_revision(revision))
        {
            return self.published_package_entry(git, revision).await;
        }
        let row = sqlx::query(
            "SELECT git, rev, title, summary, license, tags, runtime, capabilities FROM marketplace_entries WHERE source = $1 AND git = $2",
        )
        .bind(PUBLISHED_REGISTRY_SOURCE)
        .bind(git)
        .fetch_optional(&self.pool)
        .await
        .context("查询数据库已发布插件元数据失败")?;
        row.map(marketplace_entry_from_row)
            .transpose()
            .context("解析数据库已发布插件元数据失败")
            .map(|entry| entry.filter(|entry| revision_matches(&entry.rev, requested_revision)))
    }

    pub async fn marketplace_entries(&self, source: &str) -> Result<Vec<MarketplaceEntry>> {
        let rows = sqlx::query(
            "SELECT git, rev, title, summary, license, tags, runtime, capabilities FROM marketplace_entries WHERE source = $1 ORDER BY title, git",
        )
        .bind(source)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(marketplace_entry_from_row).collect()
    }
}

fn revision_matches(published: &str, requested: Option<&str>) -> bool {
    requested.is_none_or(|requested| requested.eq_ignore_ascii_case(published))
}

fn marketplace_entry_from_row(row: PgRow) -> Result<MarketplaceEntry> {
    let runtime = row
        .try_get::<Option<String>, _>("runtime")?
        .map(parse_runtime)
        .transpose()?;
    Ok(MarketplaceEntry {
        parent_git: None,
        parent_title: None,
        git: row.try_get("git")?,
        rev: row.try_get("rev")?,
        title: row.try_get("title")?,
        summary: row.try_get("summary")?,
        license: row.try_get("license")?,
        tags: serde_json::from_value(row.try_get("tags")?)?,
        installed: false,
        source_id: None,
        state: None,
        active_revision: None,
        runtime,
        capabilities: serde_json::from_value(row.try_get("capabilities")?)?,
    })
}

#[cfg(test)]
#[path = "marketplace_store_tests.rs"]
mod tests;
