use anyhow::{Result, ensure};
use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, header},
    response::Response,
};
use az_plugin_delivery::Documentation;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sqlx::{PgPool, Row};

use super::super::{RuntimeState, http_error::RuntimeError, request_context::authenticate};
use crate::runtime::RuntimeResponse;

pub(super) async fn save(pool: &PgPool, revision: &str, doc: &Documentation) -> Result<()> {
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
    sqlx::query("INSERT INTO plugin_documents(revision,readme,images) VALUES($1,$2,$3) ON CONFLICT(revision) DO NOTHING")
        .bind(revision).bind(&doc.readme).bind(serde_json::to_value(&doc.images)?).execute(pool).await?;
    Ok(())
}

pub(super) async fn details(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(revision): Path<String>,
) -> Result<Json<RuntimeResponse<serde_json::Value>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    if let Some(components) = &state.components
        && let Some(data) = components.details(&revision).await?
    {
        return Ok(Json(RuntimeResponse { data }));
    }
    let package = sqlx::query("SELECT p.git,p.version,p.source_revision,p.created_at::TEXT,d.readme FROM plugin_packages p LEFT JOIN plugin_documents d ON d.revision=p.revision WHERE p.revision=$1 AND EXISTS(SELECT 1 FROM plugin_revisions r WHERE r.revision=p.revision)")
        .bind(&revision).fetch_optional(&state.store.pool).await?;
    let Some(package) = package else {
        return Ok(Json(RuntimeResponse {
            data: serde_json::json!({"readme":"", "versions":[], "builds":[]}),
        }));
    };
    let git: String = package.try_get("git")?;
    let versions = sqlx::query("SELECT p.revision,p.version,p.source_revision,p.created_at::TEXT FROM plugin_packages p WHERE git=$1 AND EXISTS(SELECT 1 FROM plugin_revisions r WHERE r.revision=p.revision) ORDER BY p.created_at DESC LIMIT 20").bind(&git).fetch_all(&state.store.pool).await?;
    let versions = versions.into_iter().map(|r| Ok::<_,sqlx::Error>(serde_json::json!({"revision":r.try_get::<String,_>("revision")?,"version":r.try_get::<String,_>("version")?,"source_revision":r.try_get::<Option<String>,_>("source_revision")?,"created_at":r.try_get::<String,_>("created_at")?}))).collect::<Result<Vec<_>,_>>()?;
    let builds = if session
        .permissions
        .iter()
        .any(|p| p == "plugin:manage" || p == "*")
    {
        sqlx::query("SELECT id,source_revision,state,error,updated_at::TEXT FROM delivery_jobs WHERE git=$1 ORDER BY id DESC LIMIT 10").bind(&git).fetch_all(&state.store.pool).await?
            .into_iter().map(|r| Ok::<_,sqlx::Error>(serde_json::json!({"id":r.try_get::<i64,_>("id")?,"source_revision":r.try_get::<String,_>("source_revision")?,"state":r.try_get::<String,_>("state")?,"error":r.try_get::<Option<String>,_>("error")?,"updated_at":r.try_get::<String,_>("updated_at")?}))).collect::<Result<Vec<_>,_>>()?
    } else {
        vec![]
    };
    Ok(Json(RuntimeResponse {
        data: serde_json::json!({
            "readme":package.try_get::<Option<String>,_>("readme")?.unwrap_or_default(),
            "version":package.try_get::<String,_>("version")?, "source_revision":package.try_get::<Option<String>,_>("source_revision")?,
            "git":git,"versions":versions,"builds":builds
        }),
    }))
}

pub(super) async fn image(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path((revision, path)): Path<(String, String)>,
) -> Result<Response, RuntimeError> {
    authenticate(&state, &headers).await?;
    let value: Option<serde_json::Value> = sqlx::query_scalar("SELECT d.images->$2 FROM plugin_documents d WHERE d.revision=$1 AND EXISTS(SELECT 1 FROM plugin_revisions r WHERE r.revision=d.revision)").bind(&revision).bind(path).fetch_optional(&state.store.pool).await?.flatten();
    let image: az_plugin_delivery::DocumentImage =
        serde_json::from_value(value.ok_or_else(|| RuntimeError::not_found("文档图片不存在"))?)?;
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
