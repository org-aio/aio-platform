use super::super::{RuntimeState, http_error::RuntimeError, request_context::authenticate};
use super::{DeliveryService, PluginDocumentation};
use crate::runtime::RuntimeResponse;
use anyhow::{Result, ensure};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, header},
    response::Response,
    routing::get,
};
use az_plugin_delivery::Documentation;
use base64::{Engine as _, engine::general_purpose::STANDARD};

pub(crate) fn router() -> Router<RuntimeState> {
    Router::new()
        .route("/api/runtime/marketplace/{revision}/details", get(details))
        .route(
            "/api/runtime/marketplace/{revision}/images/{*path}",
            get(image),
        )
}
/// 供交付提供者在保存文档前复用路径、格式与大小校验；不产生持久化副作用。
pub fn validate_documentation(doc: &Documentation) -> Result<()> {
    ensure!(
        doc.readme.len() <= 512 * 1024 && doc.images.len() <= 64,
        "文档超过限额"
    );
    let mut remaining = 10 * 1024 * 1024;
    for (path, image) in &doc.images {
        ensure!(
            !path.starts_with('/')
                && !path.contains(['\\', ':'])
                && !path.split('/').any(|p| p == ".."),
            "文档图片路径无效"
        );
        ensure!(
            ["image/png", "image/jpeg", "image/gif", "image/webp"]
                .contains(&image.content_type.as_str()),
            "不支持的文档图片格式"
        );
        let content = STANDARD.decode(&image.content_base64)?;
        ensure!(content.len() <= remaining, "文档图片超过限额");
        remaining -= content.len();
    }
    Ok(())
}
async fn details(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(revision): Path<String>,
) -> Result<Json<RuntimeResponse<serde_json::Value>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    if let Some(components) = state.optional_service::<super::super::components::Components>()?
        && let Some(data) = components.catalog.details(&revision).await?
    {
        return Ok(Json(RuntimeResponse { data }));
    }
    let mut data = match state.optional_service::<dyn PluginDocumentation>()? {
        Some(documents) => documents.details(&revision).await?,
        None => None,
    }
    .unwrap_or_else(|| serde_json::json!({"readme":"", "versions":[]}));
    let mut builds = vec![];
    if session
        .permissions
        .iter()
        .any(|p| p == "plugin:manage" || p == "*")
        && let Some(git) = data.get("git").and_then(|v| v.as_str())
        && let Some(delivery) = state.optional_service::<dyn DeliveryService>()?
    {
        builds = delivery.builds(git).await?;
    }
    data.as_object_mut()
        .ok_or_else(|| RuntimeError::bad_request("文档提供者返回无效对象"))?
        .insert("builds".into(), serde_json::to_value(builds)?);
    Ok(Json(RuntimeResponse { data }))
}
async fn image(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path((revision, path)): Path<(String, String)>,
) -> Result<Response, RuntimeError> {
    authenticate(&state, &headers).await?;
    let documents = state
        .optional_service::<dyn PluginDocumentation>()?
        .ok_or_else(|| RuntimeError::not_found("文档图片不存在"))?;
    let image = documents
        .image(&revision, &path)
        .await?
        .ok_or_else(|| RuntimeError::not_found("文档图片不存在"))?;
    let mut response = Response::new(Body::from(STANDARD.decode(image.content_base64)?));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&image.content_type)
            .map_err(|_| RuntimeError::bad_request("图片类型无效"))?,
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}
