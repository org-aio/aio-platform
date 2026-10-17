use super::super::{RuntimeState, plugin_catalog::PluginCatalog};
use crate::runtime::{MarketplaceEntry, RuntimeCatalog};
use anyhow::Result;
#[cfg(feature = "postgres")]
use sqlx::PgPool;

#[cfg(feature = "postgres")]
pub(in crate::runtime::server) async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::Executor::execute(pool, sqlx::raw_sql("CREATE TABLE IF NOT EXISTS tenant_hidden_plugin_menus (tenant_id TEXT NOT NULL, source_id TEXT NOT NULL, PRIMARY KEY(tenant_id,source_id))")).await?;
    Ok(())
}

pub(in crate::runtime::server) async fn enrich_entries(
    store: &dyn PluginCatalog,
    tenant: &str,
    entries: &mut [MarketplaceEntry],
) -> Result<()> {
    let hidden = store.hidden_sources(tenant).await?;
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
    let hidden = state.store()?.hidden_sources(tenant).await?;
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
    catalog
        .hidden_pages
        .extend(state.store()?.hidden_page_ids(tenant, &hidden).await?);
    catalog.hidden_pages.sort();
    catalog.hidden_pages.dedup();
    Ok(())
}
