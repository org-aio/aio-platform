use super::super::{
    RuntimeState,
    http_error::RuntimeError,
    request_context::{authenticate_publisher, authorize_publish_target},
};
use crate::runtime::RuntimeResponse;
use anyhow::Context;
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, Request, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};

pub(in crate::runtime::server) fn router(state: RuntimeState) -> Router<RuntimeState> {
    Router::new()
        .route(
            "/api/runtime/components/publish",
            post(publish)
                .layer(DefaultBodyLimit::max(az_plugin_bundle::MAX_ENCODED_BYTES))
                .layer(axum::middleware::from_fn_with_state(
                    state,
                    authorize_upload,
                )),
        )
        .route(
            "/api/runtime/components/bridge.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "application/javascript")],
                    az_plugin_runtime::FRONTEND_HOST,
                )
            }),
        )
        .route(
            "/api/runtime/components/assets/{token}/{*path}",
            get(super::frontend::asset).layer(super::super::frontend_delivery::compression()),
        )
        .route(
            "/api/runtime/components/{token}/request",
            post(super::frontend::request).layer(DefaultBodyLimit::max(32 * 1024 * 1024)),
        )
        .route(
            "/api/runtime/components/{token}/renew",
            post(super::frontend::renew),
        )
        .route(
            "/api/runtime/components/{revision}/documentation",
            post(documentation).layer(DefaultBodyLimit::max(512 * 1024)),
        )
}

async fn authorize_upload(
    State(state): State<RuntimeState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if request.headers().contains_key(header::CONTENT_ENCODING) {
        return RuntimeError::unsupported_media_type("不接受传输压缩").into_response();
    }
    if let Err(error) = authenticate_publisher(&state, request.headers()).await {
        return error.into_response();
    }
    let Ok(_permit) = state.publication_slots.clone().try_acquire_owned() else {
        return RuntimeError::unavailable("正在验证其他发布，请稍后重试").into_response();
    };
    next.run(request).await
}

async fn publish(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<RuntimeResponse<serde_json::Value>>, RuntimeError> {
    let publisher = authenticate_publisher(&state, &headers).await?;
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        != Some("application/vnd.aio.component+gzip")
    {
        return Err(RuntimeError::unsupported_media_type("必须上传原生 v2 整包"));
    }
    let bundle = tokio::task::spawn_blocking(move || az_plugin_bundle::Bundle::decode(&body))
        .await
        .context("等待整包验证失败")??;
    authorize_publish_target(publisher, &bundle.git, None)?;
    let digest = bundle.digest.clone();
    let source = state.components()?.publish(bundle, "").await?;
    Ok(Json(RuntimeResponse {
        data: serde_json::json!({"source_id":source,"revision":digest,"state":"published"}),
    }))
}

async fn documentation(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(revision): Path<String>,
    body: String,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    let publisher = authenticate_publisher(&state, &headers).await?;
    let git:String=sqlx::query_scalar("SELECT s.git FROM component_sources s JOIN component_versions v ON v.source_id=s.id WHERE v.digest=$1").bind(&revision).fetch_optional(&state.store.pool).await?.context("发布版本不存在")?;
    authorize_publish_target(publisher, &git, None)?;
    sqlx::query("UPDATE component_versions SET readme=$2 WHERE digest=$1 AND readme=''")
        .bind(revision)
        .bind(body)
        .execute(&state.store.pool)
        .await?;
    Ok(Json(RuntimeResponse { data: () }))
}
