use super::{
    WebviewProtocols,
    model::{Access, OpenView, ViewRequest},
};
use crate::generated::worker_webview::WorkerWebviewService as RuntimeWorkerWebviewService;
use crate::{
    generated::worker::controller::device,
    runtime::{
        RuntimeResponse,
        server::{RuntimeState, http_error::RuntimeError},
    },
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::SinkExt;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};

pub(crate) fn router() -> Router<RuntimeState> {
    Router::new()
        .route("/api/runtime/workers/webviews/access", post(access))
        .route("/api/runtime/workers/webviews/channel", get(worker_channel))
        .route(
            "/api/runtime/components/assets/{token}/__device_view",
            post(view_request).options(crate::runtime::server::transport::preflight),
        )
        .route(
            "/api/runtime/components/assets/{token}/__device_view/{id}/__channel",
            get(browser_channel),
        )
        .route(
            "/api/runtime/components/assets/{token}/__device_view/{id}/{*path}",
            get(asset).options(crate::runtime::server::transport::preflight),
        )
}

fn response(value: Value) -> Response {
    let mut output = Json(RuntimeResponse { data: value }).into_response();
    output.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    output
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    output
}

async fn view_request(
    State(state): State<RuntimeState>,
    Path(token): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, RuntimeError> {
    let owner = state.device_view_owner(&token).await?;
    let request: ViewRequest = serde_json::from_slice(&body)?;
    match request {
        ViewRequest::List => Ok(response(serde_json::to_value(
            state
                .service::<dyn RuntimeWorkerWebviewService>()?
                .devices(&owner)
                .await?,
        )?)),
        ViewRequest::Open { device, route } => {
            let route = route.as_deref().unwrap_or("/");
            let id = state
                .service::<dyn RuntimeWorkerWebviewService>()?
                .create(&owner, &device, route)
                .await?;
            let mut src = reqwest::Url::parse(&format!(
                "{}/api/runtime/components/assets/{token}/__device_view/{id}/index.html",
                state.transport.request_origin(&headers)
            ))?;
            src.query_pairs_mut().append_pair("initialRoute", route);
            let src = src.to_string();
            Ok(response(serde_json::to_value(OpenView { id, src })?))
        }
        ViewRequest::Close { id } => {
            state
                .service::<dyn RuntimeWorkerWebviewService>()?
                .close(&owner, &id)
                .await?;
            Ok(response(Value::Null))
        }
    }
}

async fn access(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<Access>,
) -> Result<Response, RuntimeError> {
    let identity = device(&state, &headers).await?;
    let runtime = state
        .service::<WebviewProtocols>()?
        .runtime(&request.runtime)
        .map_err(|_| RuntimeError::forbidden("设备通道协议未装配"))?;
    state
        .service::<dyn RuntimeWorkerWebviewService>()?
        .access(&identity, request.enabled, request.headless, &runtime)
        .await?;
    Ok(response(Value::Null))
}

async fn worker_channel(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, RuntimeError> {
    let identity = device(&state, &headers).await?;
    let service = state.service::<dyn RuntimeWorkerWebviewService>()?;
    let tasks = state.plugin_tasks::<crate::runtime::server::plugins::WorkerWebviewsPlugin>()?;
    let capabilities = state.service::<WebviewProtocols>()?.capabilities()?;
    let (generation, incoming) = service.register(&identity, &capabilities).await?;
    Ok(upgrade
        .max_message_size(48 * 1024 * 1024)
        .max_frame_size(48 * 1024 * 1024)
        .on_upgrade(move |socket| async move {
            let cleanup = service.clone();
            let device_id = identity.id.clone();
            let connection = generation.clone();
            if let Err(error) = tasks.spawn(async move {
                worker_socket(
                    state,
                    service,
                    headers,
                    identity.id,
                    generation,
                    incoming,
                    socket,
                )
                .await;
                Ok(())
            }) {
                eprintln!("设备通道任务未启动: {error:#}");
                cleanup.unregister(&device_id, &connection).await;
            }
        }))
}

async fn worker_socket(
    state: RuntimeState,
    service: Arc<dyn RuntimeWorkerWebviewService>,
    headers: HeaderMap,
    id: String,
    generation: String,
    mut incoming: tokio::sync::mpsc::Receiver<Value>,
    mut socket: WebSocket,
) {
    let Ok(tasks) = state.plugin_tasks::<crate::runtime::server::plugins::WorkerWebviewsPlugin>()
    else {
        service.unregister(&id, &generation).await;
        return;
    };
    let cancel = tasks.cancellation();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
    let mut last_seen = tokio::time::Instant::now();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = heartbeat.tick() => {
                let authorized = channel_authorized(&state, &headers).await;
                if !authorized || last_seen.elapsed() > Duration::from_secs(45) { break; }
                if service.expire(&id).await.is_err() { break; }
                if !send(&mut socket, Message::Ping(Bytes::new()), &cancel).await { break; }
            }
            frame = incoming.recv() => {
                let Some(frame) = frame else { break; };
                if !send(&mut socket, Message::Text(frame.to_string().into()), &cancel).await { break; }
            }
            frame = socket.recv() => {
                let Some(Ok(frame)) = frame else { break; };
                last_seen = tokio::time::Instant::now();
                match frame {
                    Message::Pong(_) | Message::Ping(_) => {}
                    Message::Text(text) => {
                        let authorized = channel_authorized(&state, &headers).await;
                        if !authorized { break; }
                        let Ok(frame) = serde_json::from_str(&text) else { break; };
                        if service.receive(&id, &generation, frame).await.is_err() { break; }
                    }
                    _ => break,
                }
            }
        }
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.close()).await;
    service.unregister(&id, &generation).await;
}

async fn channel_authorized(state: &RuntimeState, headers: &HeaderMap) -> bool {
    let Ok(identity) = device(state, headers).await else {
        return false;
    };
    state
        .service::<WebviewProtocols>()
        .and_then(|protocols| protocols.enabled(&identity.capabilities))
        .unwrap_or(false)
}

async fn browser_channel(
    State(state): State<RuntimeState>,
    Path((token, id)): Path<(String, String)>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, RuntimeError> {
    // 沙箱使用 opaque origin；视图仍须持有已登录、当前版本的挂载凭据。
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok());
    if !matches!(origin, Some("null"))
        && !origin.is_some_and(|value| state.transport.accepts(value))
    {
        return Err(RuntimeError::forbidden("网页通道来源无效"));
    }
    let owner = state.device_view_owner(&token).await?;
    let service = state.service::<dyn RuntimeWorkerWebviewService>()?;
    let tasks = state.plugin_tasks::<crate::runtime::server::plugins::WorkerWebviewsPlugin>()?;
    let incoming = service.attach(&owner, &id).await?;
    Ok(upgrade
        .max_message_size(16 * 1024 * 1024)
        .on_upgrade(move |socket| async move {
            let cleanup = service.clone();
            let initial_owner = owner.clone();
            let view = id.clone();
            if let Err(error) = tasks.spawn(async move {
                browser_socket(state, service, owner, token, id, incoming, socket).await;
                Ok(())
            }) {
                eprintln!("浏览器通道任务未启动: {error:#}");
                let _ = cleanup.close(&initial_owner, &view).await;
            }
        }))
}

async fn browser_socket(
    state: RuntimeState,
    service: Arc<dyn RuntimeWorkerWebviewService>,
    initial_owner: super::model::ViewOwner,
    token: String,
    id: String,
    mut incoming: tokio::sync::mpsc::Receiver<super::service::ViewFrame>,
    mut socket: WebSocket,
) {
    let Ok(tasks) = state.plugin_tasks::<crate::runtime::server::plugins::WorkerWebviewsPlugin>()
    else {
        let _ = service.close(&initial_owner, &id).await;
        return;
    };
    let cancel = tasks.cancellation();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
    let mut last_seen = tokio::time::Instant::now();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = heartbeat.tick() => {
                if last_seen.elapsed() > Duration::from_secs(45) || state.renew_device_view(&token, &id).await.is_err() { break; }
                if !send(&mut socket, Message::Ping(Bytes::new()), &cancel).await { break; }
            }
            frame = incoming.recv() => {
                let Some(frame) = frame else { break; };
                let Ok(owner) = state.device_view_owner(&token).await else { break; };
                if service.authorize(&owner, &id).await.is_err() { break; }
                if !send(&mut socket, Message::Text(frame.value.to_string().into()), &cancel).await { break; }
            }
            frame = socket.recv() => {
                let Some(Ok(frame)) = frame else { break; };
                last_seen = tokio::time::Instant::now();
                match frame {
                    Message::Ping(_) | Message::Pong(_) => {}
                    Message::Text(text) => {
                        let Ok(owner) = state.device_view_owner(&token).await else { break; };
                        let Ok(frame) = serde_json::from_str(&text) else { break; };
                        if service.frame(&owner, &id, frame).await.is_err() { break; }
                    }
                    _ => break,
                }
            }
        }
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.close()).await;
    let _ = service.close(&initial_owner, &id).await;
}

// 慢对端不能让插件排空永久等待；取消只中断网络发送，持久化步骤仍顺序完成。
async fn send(
    socket: &mut WebSocket,
    message: Message,
    cancel: &tokio_util::sync::CancellationToken,
) -> bool {
    tokio::select! {
        _ = cancel.cancelled() => false,
        result = tokio::time::timeout(Duration::from_secs(10), socket.send(message)) => {
            matches!(result, Ok(Ok(())))
        }
    }
}

async fn asset(
    State(state): State<RuntimeState>,
    Path((token, id, path)): Path<(String, String, String)>,
    request_headers: HeaderMap,
) -> Result<Response, RuntimeError> {
    let owner = state.device_view_owner(&token).await?;
    let asset = state
        .service::<dyn RuntimeWorkerWebviewService>()?
        .asset(&owner, &id, &path)
        .await?;
    if asset.get("error").is_some() {
        return Err(RuntimeError::unavailable("设备不能提供该资源"));
    }
    let bytes = STANDARD.decode(
        asset
            .get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| RuntimeError::bad_request("资源正文缺失"))?,
    )?;
    if bytes.len() > 32 * 1024 * 1024
        || asset.get("sha256").and_then(Value::as_str)
            != Some(format!("{:x}", Sha256::digest(&bytes)).as_str())
    {
        return Err(RuntimeError::bad_request("设备资源校验失败"));
    }
    // 资源返回前再次校验，撤权不能通过进行中的资源下载绕过。
    state.device_view_owner(&token).await?;
    state
        .service::<dyn RuntimeWorkerWebviewService>()?
        .authorize(&owner, &id)
        .await?;
    let content_type = asset
        .get("contentType")
        .and_then(Value::as_str)
        .ok_or_else(|| RuntimeError::bad_request("资源类型缺失"))?;
    let prefix = format!(
        "{}/api/runtime/components/assets/{token}/__device_view/{id}/",
        state.transport.request_origin(&request_headers)
    );
    let websocket_prefix = prefix
        .replacen("https:", "wss:", 1)
        .replacen("http:", "ws:", 1);
    let mut response = bytes.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(content_type)?);
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store, no-transform"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    let document = owner
        .runtime
        .document
        .as_ref()
        .ok_or_else(|| RuntimeError::forbidden("该设备通道不提供网页文档"))?;
    let evaluation = if document.allow_eval {
        " 'unsafe-eval'"
    } else {
        ""
    };
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_str(&format!("default-src 'none'; script-src 'unsafe-inline'{evaluation} 'wasm-unsafe-eval' {prefix} blob:; connect-src {prefix} {websocket_prefix} blob:; style-src 'unsafe-inline' {prefix}; img-src data: blob: {prefix}; font-src data: {prefix}; worker-src data: {prefix}; object-src 'none'; frame-src 'none'; form-action 'none'; base-uri {prefix}"))?);
    Ok(response)
}
