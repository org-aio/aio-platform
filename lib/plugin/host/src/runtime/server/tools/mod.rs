mod controller;
mod documents;
mod model;
#[cfg(test)]
mod registration_tests;
mod storage;
#[cfg(test)]
mod tests;

use super::{RuntimeState, http_error::RuntimeError};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    routing::{get, post},
};
pub(super) use model::MarketplaceItem;
pub(super) use storage::{entries, migrate};

pub(super) fn router() -> Router<RuntimeState> {
    Router::new()
        .route("/api/runtime/tools/access", get(controller::access))
        .route("/api/runtime/tools/register", post(controller::register))
        .route(
            "/api/runtime/tools/{id}/details",
            get(controller::details).patch(controller::update),
        )
        .route("/api/runtime/tools/{id}/{version}", get(manifest))
        .layer(DefaultBodyLimit::max(az_tool::MAX_MANIFEST_BYTES as usize))
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
