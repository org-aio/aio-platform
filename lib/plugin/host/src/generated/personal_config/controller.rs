use super::model::*;
use crate::runtime::{
    RuntimeResponse,
    server::{RuntimeState, http_error::RuntimeError, request_context::authenticate},
};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, OriginalUri, Path, Query, Request, State},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde::Deserialize;
use serde_json::Value;

pub(crate) fn router(state: RuntimeState) -> Router<RuntimeState> {
    let routes = Router::new()
        .route("/catalog", get(catalog))
        .route("/entries", post(write))
        .route("/entries/{id}", get(read))
        .route("/entries/{id}/history", get(history))
        .route("/changes", get(changes))
        .route("/report", post(report))
        .route("/resolve", post(resolve))
        .route("/self", put(access_self))
        .route("/devices/{id}", put(access));
    Router::new()
        .nest("/api/runtime/personal-config", routes.clone())
        .nest("/api/runtime/workers/personal-config", routes)
        .layer(DefaultBodyLimit::max(600_000))
        .layer(middleware::from_fn_with_state(state, authorize))
}
async fn authorize(
    State(state): State<RuntimeState>,
    mut request: Request,
    next: Next,
) -> Result<Response, RuntimeError> {
    let path = request
        .extensions()
        .get::<OriginalUri>()
        .map(|uri| uri.0.path())
        .unwrap_or_else(|| request.uri().path());
    let owner = if path.starts_with("/api/runtime/workers/") {
        let device =
            crate::generated::worker::controller::device(&state, request.headers()).await?;
        if !path.ends_with("/self") && !device.capabilities.iter().any(|c| c == "config.sync") {
            return Err(RuntimeError::forbidden("设备未启用个人配置同步"));
        }
        Owner {
            tenant: device.tenant,
            user: device.user,
            device: Some(device.id),
        }
    } else {
        let session = authenticate(&state, request.headers()).await?;
        Owner {
            tenant: session.tenant_id,
            user: session.user_id,
            device: None,
        }
    };
    request.extensions_mut().insert(owner);
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    Ok(response)
}
fn response<T: serde::Serialize>(value: T) -> Response {
    Json(RuntimeResponse { data: value }).into_response()
}
async fn catalog(
    State(state): State<RuntimeState>,
    Extension(owner): Extension<Owner>,
) -> Result<Response, RuntimeError> {
    Ok(response(state.personal_config.catalog(&owner).await?))
}
#[derive(Deserialize)]
struct ReadVersion {
    revision: Option<i64>,
}
async fn read(
    State(state): State<RuntimeState>,
    Extension(owner): Extension<Owner>,
    Path(id): Path<String>,
    Query(query): Query<ReadVersion>,
) -> Result<Response, RuntimeError> {
    Ok(response(
        state
            .personal_config
            .read(&owner, &id, query.revision)
            .await?,
    ))
}
async fn history(
    State(state): State<RuntimeState>,
    Extension(owner): Extension<Owner>,
    Path(id): Path<String>,
) -> Result<Response, RuntimeError> {
    Ok(response(state.personal_config.history(&owner, &id).await?))
}
async fn write(
    State(state): State<RuntimeState>,
    Extension(owner): Extension<Owner>,
    Json(request): Json<WriteEntry>,
) -> Result<Response, RuntimeError> {
    Ok(response(
        state.personal_config.write(&owner, request).await?,
    ))
}
async fn report(
    State(state): State<RuntimeState>,
    Extension(owner): Extension<Owner>,
    Json(request): Json<Value>,
) -> Result<Response, RuntimeError> {
    state.personal_config.report(&owner, request).await?;
    Ok(response(()))
}
async fn resolve(
    State(state): State<RuntimeState>,
    Extension(owner): Extension<Owner>,
    Json(request): Json<Resolution>,
) -> Result<Response, RuntimeError> {
    state.personal_config.resolve(&owner, request).await?;
    Ok(response(()))
}
async fn access_self(
    State(state): State<RuntimeState>,
    Extension(owner): Extension<Owner>,
    Json(request): Json<Access>,
) -> Result<Response, RuntimeError> {
    let device = owner
        .device
        .as_ref()
        .ok_or_else(|| RuntimeError::forbidden("需要设备身份"))?;
    state
        .personal_config
        .access(&owner, device, request.enabled)
        .await?;
    Ok(response(()))
}
async fn access(
    State(state): State<RuntimeState>,
    Extension(owner): Extension<Owner>,
    Path(id): Path<String>,
    Json(request): Json<Access>,
) -> Result<Response, RuntimeError> {
    state
        .personal_config
        .access(&owner, &id, request.enabled)
        .await?;
    Ok(response(()))
}
async fn changes(
    State(state): State<RuntimeState>,
    Extension(owner): Extension<Owner>,
    Query(request): Query<Changes>,
) -> Result<Response, RuntimeError> {
    if request.wait > 25 || request.after < 0 {
        return Err(RuntimeError::bad_request("变更等待参数无效"));
    }
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_secs(u64::from(request.wait));
    loop {
        if let Some(device) = &owner.device {
            let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM worker_devices WHERE id=$1 AND tenant_id=$2 AND user_id=$3 AND state='active' AND capabilities ? 'config.sync')").bind(device).bind(&owner.tenant).bind(&owner.user).fetch_one(&state.store.pool).await?;
            if !active {
                return Err(RuntimeError::forbidden("设备未启用个人配置同步"));
            }
        }
        if !state
            .identity
            .member_active(&owner.tenant, &owner.user)
            .await?
        {
            return Err(RuntimeError::forbidden("设备所属账号已停用"));
        }
        let revision = state.personal_config.revision(&owner).await?;
        if revision != request.after || tokio::time::Instant::now() >= deadline {
            return Ok(response(revision));
        }
        tokio::time::sleep_until(std::cmp::min(
            deadline,
            tokio::time::Instant::now() + std::time::Duration::from_secs(1),
        ))
        .await;
    }
}
