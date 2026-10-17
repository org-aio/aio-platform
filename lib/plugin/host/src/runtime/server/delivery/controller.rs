use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use az_plugin_delivery::{BuildJob, BuildReport};
use serde::Deserialize;

use super::super::{RuntimeState, http_error::RuntimeError};
use super::{DeliveryArtifact, DeliveryService};

pub(in crate::runtime::server) fn router() -> Router<RuntimeState> {
    Router::new()
        .route("/api/internal/delivery/claim", post(claim))
        .route("/api/internal/delivery/retained-jobs", post(retained_jobs))
        .route("/api/internal/delivery/jobs/{id}/defer", post(defer))
        .route(
            "/api/internal/delivery/jobs/{id}/heartbeat",
            post(heartbeat),
        )
        .route(
            "/api/internal/delivery/jobs/{id}/package",
            post(upload).layer(DefaultBodyLimit::max(
                az_plugin_bundle::MAX_ENCODED_BYTES.max(az_plugin_package::MAX_PACKAGE_BYTES),
            )),
        )
        .route(
            "/api/internal/delivery/jobs/{id}/complete",
            post(complete).layer(DefaultBodyLimit::max(16 * 1024 * 1024)),
        )
        .route("/api/runtime/delivery/jobs/{id}/retry", post(retry))
}

fn authorize(headers: &HeaderMap) -> Result<(), RuntimeError> {
    let configured = std::env::var("AIO_DELIVERY_TOKEN").unwrap_or_default();
    let provided = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    use sha2::{Digest, Sha256};
    if configured.len() < 32
        || Sha256::digest(configured.as_bytes()) != Sha256::digest(provided.as_bytes())
    {
        return Err(RuntimeError::unauthorized("构建服务凭据无效"));
    }
    Ok(())
}

async fn claim(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<Json<Option<BuildJob>>, RuntimeError> {
    authorize(&headers)?;
    Ok(Json(state.service::<dyn DeliveryService>()?.claim().await?))
}
async fn retained_jobs(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<Json<Vec<i64>>, RuntimeError> {
    authorize(&headers)?;
    Ok(Json(
        state
            .service::<dyn DeliveryService>()?
            .retained_jobs()
            .await?,
    ))
}
#[derive(Deserialize)]
struct Lease {
    lease: String,
}
#[derive(Deserialize)]
struct DeferredBuild {
    lease: String,
    error: String,
}
async fn defer(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(request): Json<DeferredBuild>,
) -> Result<StatusCode, RuntimeError> {
    authorize(&headers)?;
    let updated = state
        .service::<dyn DeliveryService>()?
        .defer(id, &request.lease, &request.error)
        .await?;
    Ok(if updated {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::CONFLICT
    })
}
async fn heartbeat(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(request): Json<Lease>,
) -> Result<StatusCode, RuntimeError> {
    authorize(&headers)?;
    let updated = state
        .service::<dyn DeliveryService>()?
        .heartbeat(id, &request.lease)
        .await?;
    Ok(if updated {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::CONFLICT
    })
}
async fn upload(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    body: Bytes,
) -> Result<StatusCode, RuntimeError> {
    authorize(&headers)?;
    let lease = headers
        .get("x-aio-build-lease")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let format = match headers.get("content-type").and_then(|v| v.to_str().ok()) {
        Some("application/vnd.aio.component+gzip") => DeliveryArtifact::Component,
        _ => DeliveryArtifact::Plugin,
    };
    state
        .service::<dyn DeliveryService>()?
        .upload(id, lease, format, &body)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn complete(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(report): Json<BuildReport>,
) -> Result<StatusCode, RuntimeError> {
    authorize(&headers)?;
    state
        .service::<dyn DeliveryService>()?
        .complete(id, report)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn retry(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, RuntimeError> {
    super::super::request_context::authenticate_publish_manager(&state, &headers).await?;
    if !state.service::<dyn DeliveryService>()?.retry(id).await? {
        return Err(RuntimeError::bad_request("仅可重试当前目标提交的失败任务"));
    }
    Ok(StatusCode::NO_CONTENT)
}
