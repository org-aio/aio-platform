use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use az_plugin_delivery::{BuildJob, BuildReport};
use az_plugin_package::PluginPackage;
use serde::Deserialize;

use super::super::{RuntimeState, http_error::RuntimeError};
use super::{documents, store};

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
            post(upload).layer(DefaultBodyLimit::max(az_plugin_package::MAX_PACKAGE_BYTES)),
        )
        .route(
            "/api/internal/delivery/jobs/{id}/complete",
            post(complete).layer(DefaultBodyLimit::max(16 * 1024 * 1024)),
        )
        .route(
            "/api/runtime/marketplace/{revision}/details",
            get(documents::details),
        )
        .route(
            "/api/runtime/marketplace/{revision}/images/{*path}",
            get(documents::image),
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
    Ok(Json(store::claim(&state.store.pool).await?))
}

async fn retained_jobs(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<Json<Vec<i64>>, RuntimeError> {
    authorize(&headers)?;
    Ok(Json(sqlx::query_scalar("SELECT j.id FROM delivery_jobs j JOIN delivery_sources s ON s.git=j.git AND s.desired_sha=j.source_revision WHERE s.enabled AND j.state IN ('queued','building','uploaded','publishing')")
        .fetch_all(&state.store.pool).await?))
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
    let updated = sqlx::query("UPDATE delivery_jobs j SET state='queued',error=$3,lease=NULL,lease_until=NULL,retry_count=retry_count+1,next_attempt_at=now()+make_interval(secs=>LEAST(300,30*power(2,LEAST(retry_count,4)))::double precision),updated_at=now() WHERE id=$1 AND lease=$2 AND lease_until>now() AND state IN ('building','uploaded') AND EXISTS(SELECT 1 FROM delivery_sources s WHERE s.git=j.git AND s.desired_sha=j.source_revision AND s.enabled)")
        .bind(id).bind(request.lease).bind(request.error.chars().take(16000).collect::<String>())
        .execute(&state.store.pool).await?.rows_affected();
    Ok(if updated == 1 {
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
    if store::check_lease(&state.store.pool, id, &request.lease)
        .await
        .is_err()
    {
        return Ok(StatusCode::CONFLICT);
    }
    sqlx::query("UPDATE delivery_jobs SET lease_until=now()+interval '5 minutes',updated_at=now() WHERE id=$1 AND lease=$2")
        .bind(id).bind(request.lease).execute(&state.store.pool).await?;
    Ok(StatusCode::NO_CONTENT)
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
    let job = store::check_lease(&state.store.pool, id, lease).await?;
    let package = tokio::task::spawn_blocking(move || PluginPackage::decode(&body))
        .await
        .map_err(|e| RuntimeError::bad_request(e.to_string()))??;
    if package.git != job.git
        || package.source_revision.as_deref() != Some(&job.source_revision)
        || package.version != job.version
    {
        return Err(RuntimeError::bad_request(
            "构建产物与任务的仓库、源码提交或版本不符",
        ));
    }
    state.repository.stage_publish(&package).await?;
    state.store.save_package(&package).await?;
    sqlx::query("UPDATE delivery_jobs SET package_revision=$3,state='uploaded',lease_until=now()+interval '5 minutes',updated_at=now() WHERE id=$1 AND lease=$2")
        .bind(id).bind(lease).bind(package.rev).execute(&state.store.pool).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn complete(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(report): Json<BuildReport>,
) -> Result<StatusCode, RuntimeError> {
    authorize(&headers)?;
    let job = store::check_lease(&state.store.pool, id, &report.lease).await?;
    if let Some(error) = report.error {
        sqlx::query("UPDATE delivery_jobs SET state='failed',error=$3,lease_until=NULL,updated_at=now() WHERE id=$1 AND lease=$2")
            .bind(id).bind(report.lease).bind(error.chars().take(16000).collect::<String>()).execute(&state.store.pool).await?;
        return Ok(StatusCode::NO_CONTENT);
    }
    let revision: Option<String> =
        sqlx::query_scalar("SELECT package_revision FROM delivery_jobs WHERE id=$1")
            .bind(id)
            .fetch_one(&state.store.pool)
            .await?;
    let revision = revision.ok_or_else(|| RuntimeError::bad_request("尚未上传构建包"))?;
    documents::save(&state.store.pool, &revision, &report.documentation).await?;
    state.restore_package_cache(&revision).await?;
    let staged = state
        .repository
        .validate_published(&job.git, &revision)
        .await?;
    let source_id = state
        .store
        .source_id(&job.git)
        .await?
        .unwrap_or(staged.source_id);
    let publication = state
        .store
        .queue_publish_job(
            "default",
            &source_id,
            &job.git,
            &revision,
            staged.runtime,
            staged.pages.len(),
        )
        .await?;
    sqlx::query("UPDATE delivery_jobs SET state='publishing',lease_until=NULL,updated_at=now() WHERE id=$1 AND lease=$2")
        .bind(id).bind(report.lease).execute(&state.store.pool).await?;
    state.start_publish_job(publication);
    Ok(StatusCode::NO_CONTENT)
}

async fn retry(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, RuntimeError> {
    super::super::request_context::authenticate_publish_manager(&state, &headers).await?;
    let row = sqlx::query("UPDATE delivery_jobs j SET state='queued',error=NULL,lease=NULL,lease_until=NULL,retry_count=0,next_attempt_at=now(),updated_at=now() WHERE id=$1 AND state='failed' AND EXISTS(SELECT 1 FROM delivery_sources s WHERE s.git=j.git AND s.desired_sha=j.source_revision AND s.enabled) RETURNING id").bind(id).fetch_optional(&state.store.pool).await?;
    if row.is_none() {
        return Err(RuntimeError::bad_request("仅可重试当前目标提交的失败任务"));
    }
    Ok(StatusCode::NO_CONTENT)
}
