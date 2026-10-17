use super::super::enhancement_catalog::{Activation, Head, Revision};
use super::store::Enhancements;
use crate::runtime::server::{
    RuntimeState, http_error::RuntimeError, request_context::authenticate_manager,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::HeaderMap,
    routing::{get, post, put},
};
use az_plugin_enhancement::Package;

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
        .propose(
            &session.tenant_id,
            &session.user_id,
            package,
            authorize(&state, &headers, &session),
        )
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
            request,
            rollback,
            authorize(&state, &headers, &session),
        )
        .await?
        .ok_or_else(|| RuntimeError::conflict("增强活动版本已变化，请刷新后重新审阅"))?;
    Ok(Json(head))
}

async fn authorize(
    state: &RuntimeState,
    headers: &HeaderMap,
    original: &crate::identity::SessionContext,
) -> anyhow::Result<()> {
    let current = authenticate_manager(state, headers)
        .await
        .map_err(|_| anyhow::anyhow!("增强管理权限已撤销"))?;
    anyhow::ensure!(
        current.session_id == original.session_id
            && current.tenant_id == original.tenant_id
            && current.user_id == original.user_id
            && state
                .service::<dyn crate::identity::IdentityProvider>()?
                .session_active(&current.session_id, &current.tenant_id, &current.user_id)
                .await?,
        "增强管理会话已变化或撤销"
    );
    Ok(())
}
