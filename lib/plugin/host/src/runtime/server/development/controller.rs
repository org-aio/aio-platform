use super::super::{RuntimeState, http_error::RuntimeError};
use anyhow::{Context, ensure};
use axum::{
    Json, Router,
    extract::State,
    http::HeaderMap,
    routing::{get, post},
};
use az_plugin_development::{DevArtifact, DevStatus};

/// 开发控制面只由回环开发宿主装配，生产 router 没有这些路由。
pub fn router(state: RuntimeState) -> anyhow::Result<Router> {
    ensure!(
        state.config.development.is_some(),
        "生产宿主不开放开发控制面"
    );
    Ok(Router::new()
        .route("/api/development/activate", post(activate))
        .route("/api/development/prepare", post(prepare))
        .route("/api/development/status", get(status).post(update_status))
        .route("/api/development/events", get(events))
        .with_state(state))
}

fn authorize(state: &RuntimeState, headers: &HeaderMap) -> anyhow::Result<()> {
    let session = state
        .config
        .development
        .as_ref()
        .context("开发会话不存在")?;
    ensure!(
        headers.get("authorization").and_then(|h| h.to_str().ok())
            == Some(format!("Bearer {}", session.token).as_str()),
        "开发控制令牌无效"
    );
    Ok(())
}

async fn activate(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(artifact): Json<DevArtifact>,
) -> Result<Json<serde_json::Value>, RuntimeError> {
    authorize(&state, &headers)?;
    let revision = super::activation::activate(&state, artifact).await?;
    Ok(Json(serde_json::json!({"revision": revision})))
}

async fn status(State(state): State<RuntimeState>) -> Json<Option<DevStatus>> {
    Json(state.development.status.read().await.clone())
}

async fn update_status(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(status): Json<DevStatus>,
) -> Result<Json<bool>, RuntimeError> {
    authorize(&state, &headers)?;
    let mut current = state.development.status.write().await;
    if current
        .as_ref()
        .is_none_or(|previous| previous.generation <= status.generation)
    {
        *current = Some(status);
    }
    state.development.events.send_replace(current.clone());
    if current
        .as_ref()
        .is_some_and(|status| status.phase != "building")
    {
        if current
            .as_ref()
            .is_some_and(|status| status.phase == "failed")
        {
            state.components()?.discard_development_candidate().await;
        }
        if let Err(error) = super::retention::collect(&state).await {
            eprintln!("开发快照回收失败: {error:#}");
        }
    }
    Ok(Json(true))
}

async fn events(
    State(state): State<RuntimeState>,
) -> axum::response::Sse<
    impl futures_util::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
> {
    let receiver = state.development.events.subscribe();
    let stream =
        futures_util::stream::unfold((receiver, true), |(mut receiver, first)| async move {
            if !first && receiver.changed().await.is_err() {
                return None;
            }
            let value = receiver.borrow_and_update().clone();
            let event = axum::response::sse::Event::default()
                .json_data(value)
                .expect("开发状态可序列化");
            Some((Ok(event), (receiver, false)))
        });
    axum::response::Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

async fn prepare(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(artifact): Json<DevArtifact>,
) -> Result<Json<az_plugin_development::DevLaunch>, RuntimeError> {
    authorize(&state, &headers)?;
    Ok(Json(super::activation::prepare(&state, artifact).await?))
}
