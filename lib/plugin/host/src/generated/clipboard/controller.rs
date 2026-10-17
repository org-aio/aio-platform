use super::model::*;
use crate::generated::clipboard::ClipboardService as RuntimeClipboardService;
use crate::generated::worker::WorkerService as RuntimeWorkerService;
use crate::identity::SessionContext;
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

pub(crate) fn router(state: RuntimeState) -> Router<RuntimeState> {
    let routes = Router::new()
        .route("/head", get(head))
        .route("/items", get(list).post(write))
        .route("/items/{id}", get(read))
        .route("/items/{id}/push", post(push))
        .route("/devices", get(devices))
        .route("/self", put(access_self))
        .route("/devices/{id}", put(access))
        .route("/changes", get(changes));
    Router::new()
        .nest("/api/runtime/clipboard", routes.clone())
        .nest("/api/runtime/workers/clipboard", routes)
        .layer(DefaultBodyLimit::max(16 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(state, authorize))
}

/// 设备与网页共用同一服务；设备请求额外校验剪切板通道是否已开通。
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
        if !path.ends_with("/self") && !device.capabilities.iter().any(|c| c == "clipboard.sync") {
            return Err(RuntimeError::forbidden("设备未启用剪切板接力"));
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

async fn head(
    State(state): State<RuntimeState>,
    Extension(owner): Extension<Owner>,
) -> Result<Response, RuntimeError> {
    Ok(response(
        state
            .service::<dyn RuntimeClipboardService>()?
            .head(&owner)
            .await?,
    ))
}

#[derive(Deserialize)]
struct ListQuery {
    cursor: Option<i64>,
    limit: Option<i64>,
}

async fn list(
    State(state): State<RuntimeState>,
    Extension(owner): Extension<Owner>,
    Query(query): Query<ListQuery>,
) -> Result<Response, RuntimeError> {
    let limit = query.limit.unwrap_or(30);
    if query.cursor.is_some_and(|cursor| cursor < 0) {
        return Err(RuntimeError::bad_request("分页游标无效"));
    }
    Ok(response(
        state
            .service::<dyn RuntimeClipboardService>()?
            .list(&owner, query.cursor, limit)
            .await?,
    ))
}

async fn read(
    State(state): State<RuntimeState>,
    Extension(owner): Extension<Owner>,
    Path(id): Path<String>,
) -> Result<Response, RuntimeError> {
    Ok(response(
        state
            .service::<dyn RuntimeClipboardService>()?
            .read(&owner, &id)
            .await?,
    ))
}

async fn write(
    State(state): State<RuntimeState>,
    Extension(owner): Extension<Owner>,
    Json(request): Json<ClipWrite>,
) -> Result<Response, RuntimeError> {
    Ok(response(
        state
            .service::<dyn RuntimeClipboardService>()?
            .write(&owner, request)
            .await?,
    ))
}

/// 把指定条目显式投递到设备；未指定设备时投递到全部已开通通道的设备。
async fn push(
    State(state): State<RuntimeState>,
    Extension(owner): Extension<Owner>,
    Path(id): Path<String>,
    Json(request): Json<Push>,
) -> Result<Response, RuntimeError> {
    if request.devices.len() > 100 {
        return Err(RuntimeError::bad_request("投递设备过多"));
    }
    let content = state
        .service::<dyn RuntimeClipboardService>()?
        .read(&owner, &id)
        .await?;
    let targets = if request.devices.is_empty() {
        state
            .service::<dyn RuntimeClipboardService>()?
            .devices(&owner)
            .await?
            .into_iter()
            .filter(|device| device.enabled)
            .map(|device| device.id)
            .collect::<Vec<_>>()
    } else {
        request.devices
    };
    let session = SessionContext {
        session_id: format!("clipboard:{}", owner.user),
        user_id: owner.user.clone(),
        account: String::new(),
        display_name: String::new(),
        tenant_id: owner.tenant.clone(),
        tenant_label: String::new(),
        permissions: Vec::new(),
    };
    // 任务只携带条目 ID，设备领取后通过自己的剪切板通道读取加密正文；
    // 这样图片和二进制不受 32 KB 任务输入限制，也不会在任务队列中复制大正文。
    let input = serde_json::json!({"id": content.item.id});
    let mut tasks = Vec::new();
    for device in targets {
        let task = state
            .service::<dyn RuntimeWorkerService>()?
            .enqueue(
                &session,
                crate::generated::worker::model::SubmitTask {
                    id: uuid::Uuid::new_v4().to_string(),
                    worker_id: device,
                    capability: "clipboard.sync".into(),
                    input: input.clone(),
                },
            )
            .await?;
        tasks.push(task.id);
    }
    Ok(response(tasks))
}

async fn devices(
    State(state): State<RuntimeState>,
    Extension(owner): Extension<Owner>,
) -> Result<Response, RuntimeError> {
    Ok(response(
        state
            .service::<dyn RuntimeClipboardService>()?
            .devices(&owner)
            .await?,
    ))
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
        .service::<dyn RuntimeClipboardService>()?
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
        .service::<dyn RuntimeClipboardService>()?
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
            let active = state
                .service::<dyn crate::generated::worker::WorkerService>()?
                .capability_active(&owner.tenant, &owner.user, device, "clipboard.sync")
                .await?;
            if !active {
                return Err(RuntimeError::forbidden("设备未启用剪切板接力"));
            }
        }
        if !state
            .service::<dyn crate::identity::IdentityProvider>()?
            .member_active(&owner.tenant, &owner.user)
            .await?
        {
            return Err(RuntimeError::forbidden("设备所属账号已停用"));
        }
        let head = state
            .service::<dyn RuntimeClipboardService>()?
            .head(&owner)
            .await?;
        if head.revision != request.after || tokio::time::Instant::now() >= deadline {
            return Ok(response(head.revision));
        }
        tokio::time::sleep_until(std::cmp::min(
            deadline,
            tokio::time::Instant::now() + std::time::Duration::from_secs(1),
        ))
        .await;
    }
}
