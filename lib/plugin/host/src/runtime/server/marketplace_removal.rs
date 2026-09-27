use super::{
    RuntimeState, http_error::RuntimeError, request_context::authenticate_publish_manager,
};
use crate::runtime::{MarketplaceEntry, RuntimeResponse};
use axum::{Json, extract::State, http::HeaderMap};
use serde::Deserialize;
use sqlx::PgPool;
use std::collections::HashSet;

#[derive(Deserialize)]
pub(super) struct RemoveRequest {
    git: String,
}

// 市场下架只记录来源，不删除发布包、租户安装或业务数据。
pub(super) async fn remove(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<RemoveRequest>,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    authenticate_publish_manager(&state, &headers).await?;
    let mut exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM marketplace_entries WHERE git=$1) OR EXISTS(SELECT 1 FROM plugin_sources WHERE git=$1)",
    ).bind(&request.git).fetch_one(&state.store.pool).await?;
    if !exists && state.components.is_some() {
        exists = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM component_sources WHERE git=$1)")
            .bind(&request.git)
            .fetch_one(&state.store.pool)
            .await?;
    }
    if !exists {
        return Err(RuntimeError::not_found("插件市场条目不存在"));
    }
    mark_removed(&state.store.pool, &request.git).await?;
    Ok(Json(RuntimeResponse { data: () }))
}

async fn mark_removed(pool: &PgPool, git: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO marketplace_plugin_removals(git) VALUES($1) ON CONFLICT(git) DO NOTHING",
    )
    .bind(git)
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn is_removed(pool: &PgPool, git: &str) -> anyhow::Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM marketplace_plugin_removals WHERE git=$1)")
            .bind(git)
            .fetch_one(pool)
            .await?,
    )
}

// 下架只影响市场可安装目录；租户已安装的条目必须保留，否则会丢失停用和卸载入口。
pub(super) async fn retain_listed(
    pool: &PgPool,
    entries: &mut Vec<MarketplaceEntry>,
) -> anyhow::Result<()> {
    let removed: Vec<String> = sqlx::query_scalar("SELECT git FROM marketplace_plugin_removals")
        .fetch_all(pool)
        .await?;
    retain_installed_or_listed(&removed.into_iter().collect(), entries);
    Ok(())
}

fn retain_installed_or_listed(removed: &HashSet<String>, entries: &mut Vec<MarketplaceEntry>) {
    entries.retain(|entry| entry.installed || !removed.contains(&entry.git));
}

#[cfg(test)]
#[path = "marketplace_removal_tests.rs"]
mod tests;

#[cfg(test)]
mod unit_tests {
    use super::*;

    fn entry(git: &str, installed: bool) -> MarketplaceEntry {
        MarketplaceEntry {
            parent_git: None,
            parent_title: None,
            git: git.into(),
            rev: "rev".into(),
            title: git.into(),
            summary: String::new(),
            license: "MIT".into(),
            tags: Vec::new(),
            installed,
            menu_hidden: false,
            source_id: None,
            state: None,
            active_revision: None,
            runtime: None,
            capabilities: Default::default(),
        }
    }

    #[test]
    fn delisting_hides_only_uninstalled_entries() {
        let removed = HashSet::from(["https://example.com/removed.git".to_owned()]);
        let mut entries = vec![
            entry("https://example.com/removed.git", false),
            entry("https://example.com/removed.git", true),
            entry("https://example.com/listed.git", false),
        ];
        retain_installed_or_listed(&removed, &mut entries);
        assert_eq!(entries.len(), 2);
        assert!(
            entries
                .iter()
                .any(|e| e.installed && e.git.ends_with("removed.git"))
        );
        assert!(entries.iter().any(|e| e.git.ends_with("listed.git")));
    }

    #[test]
    fn entries_without_removal_records_are_all_kept() {
        let mut entries = vec![entry("https://example.com/listed.git", false)];
        retain_installed_or_listed(&HashSet::new(), &mut entries);
        assert_eq!(entries.len(), 1);
    }
}
