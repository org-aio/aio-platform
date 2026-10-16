use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::HeaderMap,
    routing::{get, post, put},
};
use az_plugin_enhancement::Package;
use serde::Deserialize;

use super::store::{Enhancements, Head, Revision};
use crate::runtime::server::{
    RuntimeState, http_error::RuntimeError, request_context::authenticate_manager,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Activation {
    revision: Option<String>,
    expected_generation: i64,
}

pub(in crate::runtime::server) fn router() -> Router<RuntimeState> {
    Router::new()
        .route("/api/runtime/enhancements", get(head))
        .route(
            "/api/runtime/enhancements/revisions",
            get(revisions).post(propose),
        )
        .route(
            "/api/runtime/enhancements/revisions/{revision}",
            get(package),
        )
        .route("/api/runtime/enhancements/active", put(activate))
        .route("/api/runtime/enhancements/rollback", post(rollback))
        .layer(DefaultBodyLimit::max(
            az_plugin_enhancement::MAX_PACKAGE_BYTES,
        ))
}

async fn head(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<Json<Head>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    Ok(Json(
        state
            .service::<Enhancements>()?
            .head(&session.tenant_id)
            .await?,
    ))
}

async fn revisions(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Revision>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    Ok(Json(
        state
            .service::<Enhancements>()?
            .revisions(&session.tenant_id)
            .await?,
    ))
}

async fn package(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(revision): Path<String>,
) -> Result<Json<Package>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    Ok(Json(
        state
            .service::<Enhancements>()?
            .package(&session.tenant_id, &revision)
            .await?,
    ))
}

async fn propose(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(package): Json<Package>,
) -> Result<Json<serde_json::Value>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    let revision = state
        .service::<Enhancements>()?
        .propose(&session.tenant_id, &session.user_id, package)
        .await?;
    Ok(Json(serde_json::json!({"revision": revision})))
}

async fn activate(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<Activation>,
) -> Result<Json<Head>, RuntimeError> {
    change(state, headers, request, false).await
}

async fn rollback(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<Activation>,
) -> Result<Json<Head>, RuntimeError> {
    change(state, headers, request, true).await
}

async fn change(
    state: RuntimeState,
    headers: HeaderMap,
    request: Activation,
    rollback: bool,
) -> Result<Json<Head>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    let head = state
        .service::<Enhancements>()?
        .activate(
            &session.tenant_id,
            &session.user_id,
            request.revision.as_deref(),
            request.expected_generation,
            rollback,
        )
        .await?
        .ok_or_else(|| RuntimeError::conflict("增强活动版本已变化，请刷新后重新审阅"))?;
    Ok(Json(head))
}
