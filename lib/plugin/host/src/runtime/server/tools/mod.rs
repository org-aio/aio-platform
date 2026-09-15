mod model;
mod storage;
#[cfg(test)]
mod tests;

use super::{RuntimeState, http_error::RuntimeError};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
pub(super) use model::MarketplaceItem;
pub(super) use storage::{entries, migrate};

pub(super) fn router() -> Router<RuntimeState> {
    Router::new().route("/api/runtime/tools/{id}/{version}", get(manifest))
}

async fn manifest(
    State(state): State<RuntimeState>,
    Path((id, version)): Path<(String, String)>,
) -> Result<Json<az_tool::ToolManifest>, RuntimeError> {
    az_tool::InstallLink::parse(
        &az_tool::InstallLink {
            id: id.clone(),
            version: version.clone(),
        }
        .to_string(),
    )?;
    let value = storage::get(&state.store.pool, &id, &version)
        .await?
        .ok_or_else(|| RuntimeError::not_found("该工具版本尚未收录于插件市场"))?;
    Ok(Json(value))
}
