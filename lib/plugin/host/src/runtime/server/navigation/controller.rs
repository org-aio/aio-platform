use super::super::{
    RuntimeState,
    http_error::RuntimeError,
    request_context::{authenticate_manager, catalog_value},
};
use crate::runtime::RuntimeResponse;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::post,
};

pub(in crate::runtime::server) fn router() -> Router<RuntimeState> {
    Router::new()
        .route("/api/runtime/plugins/{source_id}/hide-menu", post(hide))
        .route("/api/runtime/plugins/{source_id}/show-menu", post(show))
}

async fn hide(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(source): Path<String>,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    change(state, headers, source, true).await
}

async fn show(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(source): Path<String>,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    change(state, headers, source, false).await
}

async fn change(
    state: RuntimeState,
    headers: HeaderMap,
    source: String,
    hidden: bool,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    let catalog = catalog_value(&state, &session).await?;
    if !catalog
        .plugins
        .iter()
        .any(|plugin| plugin.source_id == source)
    {
        return Err(RuntimeError::forbidden("当前租户未安装此插件"));
    }
    super::service::set_hidden(&state.store.pool, &session.tenant_id, &source, hidden).await?;
    Ok(Json(RuntimeResponse { data: () }))
}
