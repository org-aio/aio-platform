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

// 在合并组件、传统插件和已安装条目之后过滤，避免回填导致下架条目复活。
pub(super) async fn retain_listed(
    pool: &PgPool,
    entries: &mut Vec<MarketplaceEntry>,
) -> anyhow::Result<()> {
    let removed: Vec<String> = sqlx::query_scalar("SELECT git FROM marketplace_plugin_removals")
        .fetch_all(pool)
        .await?;
    let removed: HashSet<String> = removed.into_iter().collect();
    entries.retain(|entry| !removed.contains(&entry.git));
    Ok(())
}

#[cfg(test)]
#[path = "marketplace_removal_tests.rs"]
mod tests;
