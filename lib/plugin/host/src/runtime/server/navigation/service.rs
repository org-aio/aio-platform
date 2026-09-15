use super::super::RuntimeState;
use crate::runtime::{MarketplaceEntry, PageDefinition, RuntimeCatalog};
use anyhow::Result;
use sqlx::PgPool;

pub(in crate::runtime::server) async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql("CREATE TABLE IF NOT EXISTS tenant_hidden_plugin_menus (tenant_id TEXT NOT NULL, source_id TEXT NOT NULL, PRIMARY KEY(tenant_id,source_id))")
        .execute(pool).await?;
    Ok(())
}

pub(super) async fn set_hidden(
    pool: &PgPool,
    tenant: &str,
    source: &str,
    hidden: bool,
) -> Result<()> {
    let sql = if hidden {
        "INSERT INTO tenant_hidden_plugin_menus(tenant_id,source_id) VALUES($1,$2) ON CONFLICT DO NOTHING"
    } else {
        "DELETE FROM tenant_hidden_plugin_menus WHERE tenant_id=$1 AND source_id=$2"
    };
    sqlx::query(sql)
        .bind(tenant)
        .bind(source)
        .execute(pool)
        .await?;
    Ok(())
}

async fn hidden_sources(pool: &PgPool, tenant: &str) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT source_id FROM tenant_hidden_plugin_menus WHERE tenant_id=$1 ORDER BY source_id",
    )
    .bind(tenant)
    .fetch_all(pool)
    .await?)
}

pub(in crate::runtime::server) async fn enrich_entries(
    pool: &PgPool,
    tenant: &str,
    entries: &mut [MarketplaceEntry],
) -> Result<()> {
    let hidden = hidden_sources(pool, tenant).await?;
    for entry in entries {
        entry.menu_hidden = entry
            .source_id
            .as_ref()
            .is_some_and(|source| hidden.contains(source));
    }
    Ok(())
}

pub(in crate::runtime::server) async fn apply(
    state: &RuntimeState,
    tenant: &str,
    catalog: &mut RuntimeCatalog,
) -> Result<()> {
    let hidden = hidden_sources(&state.store.pool, tenant).await?;
    if hidden.is_empty() {
        return Ok(());
    }
    // 只提供导航偏好，完整页面目录继续用于挂载与服务授权。
    for source in &hidden {
        let prefix = format!("component:{source}:");
        catalog.hidden_pages.extend(
            catalog
                .pages
                .iter()
                .filter(|page| page.id.starts_with(&prefix))
                .map(|page| page.id.clone()),
        );
    }
    let pages = sqlx::query_scalar::<_, serde_json::Value>("SELECT r.pages FROM tenant_plugin_bindings b JOIN plugin_revisions r ON r.id=b.revision_id WHERE b.tenant_id=$1 AND b.enabled AND b.source_id=ANY($2)")
        .bind(tenant).bind(&hidden).fetch_all(&state.store.pool).await?;
    for pages in pages {
        catalog.hidden_pages.extend(
            serde_json::from_value::<Vec<PageDefinition>>(pages)?
                .into_iter()
                .map(|page| page.id),
        );
    }
    catalog.hidden_pages.sort();
    catalog.hidden_pages.dedup();
    Ok(())
}
