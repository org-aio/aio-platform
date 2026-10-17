use super::super::{
    RuntimeState,
    http_error::RuntimeError,
    request_context::{authenticate, authenticate_publish_manager},
};
use super::{documents, storage};
use crate::runtime::RuntimeResponse;
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use az_tool::{
    ToolManifest,
    registration::{Documentation, Metadata, Registration},
};

pub(super) async fn remove(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    authenticate_publish_manager(&state, &headers).await?;
    storage::remove(state.service::<sqlx::PgPool>()?.as_ref(), &id).await?;
    Ok(Json(RuntimeResponse { data: () }))
}

pub(super) async fn access(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<Json<RuntimeResponse<bool>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    let allowed = session.permissions.iter().any(|p| p == "plugin:manage")
        && state
            .service::<dyn crate::authorization::AuthorizationProvider>()?
            .can_publish(&session)
            .await?;
    Ok(Json(RuntimeResponse { data: allowed }))
}

pub(super) async fn register(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<Registration>,
) -> Result<Json<RuntimeResponse<ToolManifest>>, RuntimeError> {
    authenticate_publish_manager(&state, &headers).await?;
    let mut manifest = request.manifest("pending".into())?;
    let signature = serde_json::to_vec(&manifest.platforms)?;
    manifest.id = format!(
        "cli-{}",
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, &signature).simple()
    );
    // 重复提交不会修改已有安装步骤或覆盖标题备注。
    if let Some(existing) = storage::get(
        state.service::<sqlx::PgPool>()?.as_ref(),
        &manifest.id,
        &manifest.version,
    )
    .await?
    {
        return Ok(Json(RuntimeResponse { data: existing }));
    }
    let metadata = Metadata {
        git: manifest.homepage.clone(),
        title: manifest.title.clone(),
        summary: manifest.summary.clone(),
    };
    let doc = documents::load(&state.config.cache_root, metadata).await;
    storage::register(state.service::<sqlx::PgPool>()?.as_ref(), &manifest, &doc).await?;
    Ok(Json(RuntimeResponse { data: manifest }))
}

pub(super) async fn details(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<RuntimeResponse<Documentation>>, RuntimeError> {
    authenticate(&state, &headers).await?;
    let doc = if let Some(doc) =
        storage::documentation(state.service::<sqlx::PgPool>()?.as_ref(), &id).await?
    {
        doc
    } else {
        let repository = state.repository()?;
        let _guard = repository.cache_operations.lock().await;
        if let Some(doc) =
            storage::documentation(state.service::<sqlx::PgPool>()?.as_ref(), &id).await?
        {
            doc
        } else {
            let value: Option<serde_json::Value> = sqlx::query_scalar("SELECT manifest FROM marketplace_tools WHERE id=$1 ORDER BY created_at DESC LIMIT 1")
                .bind(&id).fetch_optional(state.service::<sqlx::PgPool>()?.as_ref()).await?;
            let manifest: ToolManifest = serde_json::from_value(
                value.ok_or_else(|| RuntimeError::not_found("CLI 条目不存在"))?,
            )?;
            let metadata = Metadata {
                git: manifest.homepage,
                title: manifest.title,
                summary: manifest.summary,
            };
            let doc = documents::load(&state.config.cache_root, metadata).await;
            storage::save_documentation(state.service::<sqlx::PgPool>()?.as_ref(), &id, &doc)
                .await?;
            doc
        }
    };
    Ok(Json(RuntimeResponse { data: doc }))
}

pub(super) async fn update(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(mut metadata): Json<Metadata>,
) -> Result<Json<RuntimeResponse<Documentation>>, RuntimeError> {
    authenticate_publish_manager(&state, &headers).await?;
    metadata.normalize()?;
    if metadata.title.is_empty() {
        return Err(RuntimeError::bad_request("标题不能为空"));
    }
    if !storage::exists(state.service::<sqlx::PgPool>()?.as_ref(), &id).await? {
        return Err(RuntimeError::not_found("CLI 条目不存在"));
    }
    let doc = documents::load(&state.config.cache_root, metadata).await;
    storage::save_documentation(state.service::<sqlx::PgPool>()?.as_ref(), &id, &doc).await?;
    Ok(Json(RuntimeResponse { data: doc }))
}
