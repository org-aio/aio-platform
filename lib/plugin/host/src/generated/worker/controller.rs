use super::{archive, model::*};
use crate::runtime::{
    RuntimeResponse,
    server::{RuntimeState, http_error::RuntimeError, request_context::authenticate},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, header},
    routing::{delete, get, post, put},
};

pub(crate) fn router() -> Router<RuntimeState> {
    Router::new()
        .route("/api/runtime/workers/pairings", post(pair))
        .route("/api/runtime/workers/pairings/poll", post(poll))
        .route(
            "/api/runtime/workers/pairings/{code}",
            get(pairing).post(approve),
        )
        .route("/api/runtime/workers", get(list))
        .route("/api/runtime/workers/{id}", delete(revoke))
        .route("/api/runtime/workers/{id}/desktop", put(desktop))
        .route("/api/runtime/workers/tasks", get(tasks).post(enqueue))
        .route("/api/runtime/workers/tasks/{id}", get(task))
        .route("/api/runtime/workers/tasks/{id}/cancel", post(cancel_task))
        .route(
            "/api/runtime/workers/workspaces/access",
            post(workspace_access),
        )
        .route("/api/runtime/workers/desktop/access", post(desktop_access))
        .route("/api/runtime/workers/claim", post(claim))
        .route("/api/runtime/workers/heartbeat", post(heartbeat))
        .route("/api/runtime/workers/tasks/{id}/heartbeat", post(renew))
        .route("/api/runtime/workers/tasks/{id}/complete", post(complete))
        .layer(DefaultBodyLimit::max(600_000))
        .merge(archive::router())
}
fn response<T>(data: T) -> Json<RuntimeResponse<T>> {
    Json(RuntimeResponse { data })
}
pub(super) fn token(headers: &HeaderMap) -> Result<String, RuntimeError> {
    let raw = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| RuntimeError::unauthorized("缺少设备凭据"))?;
    if let Some(token) = raw.strip_prefix("Bearer ") {
        return Ok(token.into());
    }
    if let Some(encoded) = raw.strip_prefix("Basic ") {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)?;
        let text = String::from_utf8(bytes)?;
        if let Some(token) = text.strip_prefix("worker:") {
            return Ok(token.into());
        }
    }
    Err(RuntimeError::unauthorized("设备凭据无效"))
}
pub(crate) async fn device(
    state: &RuntimeState,
    headers: &HeaderMap,
) -> Result<DeviceIdentity, RuntimeError> {
    let identity = state
        .workers
        .identity(&token(headers)?)
        .await
        .map_err(|error| {
            if error.downcast_ref::<sqlx::Error>().is_some() {
                RuntimeError::unavailable("设备认证服务暂不可用")
            } else {
                RuntimeError::unauthorized("设备尚未配对或已撤销")
            }
        })?;
    if !state
        .identity
        .member_active(&identity.tenant, &identity.user)
        .await?
    {
        return Err(RuntimeError::forbidden("设备所属账号已停用"));
    }
    Ok(identity)
}
async fn pair(
    State(state): State<RuntimeState>,
    Json(request): Json<PairRequest>,
) -> Result<Json<RuntimeResponse<Pairing>>, RuntimeError> {
    Ok(response(state.workers.pair(request).await?))
}
async fn poll(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<Json<RuntimeResponse<String>>, RuntimeError> {
    Ok(response(state.workers.poll(&token(&headers)?).await?))
}
async fn pairing(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(code): Path<String>,
) -> Result<Json<RuntimeResponse<Worker>>, RuntimeError> {
    authenticate(&state, &headers).await?;
    Ok(response(state.workers.pairing(&code).await?))
}
async fn approve(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(code): Path<String>,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    state.workers.approve(&session, &code).await?;
    Ok(response(()))
}
async fn list(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<Json<RuntimeResponse<Vec<Worker>>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    Ok(response(state.workers.list(&session).await?))
}
async fn revoke(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    state.workers.revoke(&session, &id).await?;
    Ok(response(()))
}
async fn enqueue(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<SubmitTask>,
) -> Result<Json<RuntimeResponse<Task>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    Ok(response(state.workers.enqueue(&session, request).await?))
}
async fn tasks(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<Json<RuntimeResponse<Vec<Task>>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    Ok(response(state.workers.tasks(&session).await?))
}
async fn claim(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<ClaimRequest>,
) -> Result<Json<RuntimeResponse<Option<Task>>>, RuntimeError> {
    uuid::Uuid::parse_str(&request.request_id)?;
    if request.wait_seconds > 25 {
        return Err(RuntimeError::bad_request("长轮询最长等待 25 秒"));
    }
    let deadline = tokio::time::Instant::now()
        + std::time::Duration::from_secs(u64::from(request.wait_seconds));
    let connected = device(&state, &headers).await?;
    state
        .workers
        .heartbeat(&connected, None)
        .await
        .map_err(worker_error)?;
    loop {
        // 每轮重新鉴权，等待中的连接在撤权后也不能收到任务。
        let device = device(&state, &headers).await?;
        let task = state
            .workers
            .claim(&device, &request.request_id)
            .await
            .map_err(worker_error)?;
        if task.is_some() || tokio::time::Instant::now() >= deadline {
            return Ok(response(task));
        }
        tokio::time::sleep_until(std::cmp::min(
            deadline,
            tokio::time::Instant::now() + std::time::Duration::from_secs(1),
        ))
        .await;
    }
}

fn worker_error(error: anyhow::Error) -> RuntimeError {
    if error.downcast_ref::<sqlx::Error>().is_some() {
        RuntimeError::unavailable("设备任务服务暂不可用")
    } else {
        error.into()
    }
}

async fn task(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<RuntimeResponse<Task>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    Ok(response(state.workers.task(&session, &id).await?))
}

async fn cancel_task(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<RuntimeResponse<Task>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    Ok(response(state.workers.cancel_task(&session, &id).await?))
}

async fn workspace_access(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<WorkspaceAccess>,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    if !headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("Bearer "))
    {
        return Err(RuntimeError::unauthorized("缺少设备 Bearer 凭据"));
    }
    let device = device(&state, &headers).await?;
    state
        .workers
        .workspace_access(&device, request.enabled)
        .await
        .map_err(worker_error)?;
    Ok(response(()))
}

async fn desktop_access(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<WorkspaceAccess>,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    if !headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("Bearer "))
    {
        return Err(RuntimeError::unauthorized("缺少设备 Bearer 凭据"));
    }
    let device = device(&state, &headers).await?;
    state
        .workers
        .desktop_access(&device, request.enabled)
        .await
        .map_err(worker_error)?;
    Ok(response(()))
}

async fn desktop(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<DesktopAccess>,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    state
        .workers
        .desktop(&session, &id, request.enabled)
        .await?;
    Ok(response(()))
}
async fn heartbeat(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    let device = device(&state, &headers).await?;
    state
        .workers
        .heartbeat(&device, None)
        .await
        .map_err(worker_error)?;
    Ok(response(()))
}
async fn renew(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<Lease>,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    let device = device(&state, &headers).await?;
    state
        .workers
        .heartbeat(&device, Some((&id, &request.lease)))
        .await
        .map_err(worker_error)?;
    Ok(response(()))
}
async fn complete(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<CompleteTask>,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    let device = device(&state, &headers).await?;
    state
        .workers
        .complete(&device, &id, request)
        .await
        .map_err(worker_error)?;
    Ok(response(()))
}
