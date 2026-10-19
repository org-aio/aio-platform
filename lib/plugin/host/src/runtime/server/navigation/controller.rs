use super::super::{
    RuntimeState,
    http_error::RuntimeError,
    request_context::{authenticate_manager, catalog_value},
};
use super::{NavigationLabelKind, validate_navigation_label};
use crate::runtime::RuntimeResponse;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::post,
};
use serde::Deserialize;

pub(in crate::runtime::server) fn router() -> Router<RuntimeState> {
    Router::new()
        .route("/api/runtime/plugins/{source_id}/hide-menu", post(hide))
        .route("/api/runtime/plugins/{source_id}/show-menu", post(show))
        .route(
            "/api/runtime/navigation/labels",
            post(rename).delete(restore),
        )
}

/// 覆盖请求只接受新的显示名；类别与键来自请求体，且必须已存在于目录。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LabelRequest {
    kind: String,
    key: String,
    #[serde(default)]
    label: Option<String>,
}

async fn rename(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<LabelRequest>,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    let kind = NavigationLabelKind::parse(&request.kind)
        .ok_or_else(|| RuntimeError::not_found("未知的导航覆盖类别"))?;
    let label = request.label.unwrap_or_default();
    validate_navigation_label(kind, &request.key, &label)
        .map_err(|error| RuntimeError::bad_request(error.to_string()))?;
    // 目录来自当前会话；只有真实存在于导航中的元素才能被覆盖。
    if !addressable(&catalog_value(&state, &session).await?, kind, &request.key) {
        return Err(RuntimeError::not_found("目录中不存在该导航元素"));
    }
    state
        .store()?
        .set_navigation_label(&session.tenant_id, kind, &request.key, label.trim())
        .await?;
    Ok(Json(RuntimeResponse { data: () }))
}

async fn restore(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<LabelRequest>,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    let kind = NavigationLabelKind::parse(&request.kind)
        .ok_or_else(|| RuntimeError::not_found("未知的导航覆盖类别"))?;
    state
        .store()?
        .clear_navigation_label(&session.tenant_id, kind, &request.key)
        .await?;
    Ok(Json(RuntimeResponse { data: () }))
}

fn addressable(
    catalog: &crate::runtime::RuntimeCatalog,
    kind: NavigationLabelKind,
    key: &str,
) -> bool {
    match kind {
        NavigationLabelKind::Scene => catalog.pages.iter().any(|page| page.scene.id == key),
        NavigationLabelKind::Page => {
            catalog.pages.iter().any(|page| page.id == key)
                || catalog.account_items.iter().any(|item| item.id == key)
                || catalog
                    .plugin_settings
                    .iter()
                    .any(|item| item.page_id == key)
        }
        NavigationLabelKind::Menu => catalog
            .pages
            .iter()
            .any(|page| page.menu_path.iter().any(|group| group.id == key)),
    }
}

async fn hide(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(source): Path<String>,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    change(state, headers, source, true).await
}

async fn show(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(source): Path<String>,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    change(state, headers, source, false).await
}

async fn change(
    state: RuntimeState,
    headers: HeaderMap,
    source: String,
    hidden: bool,
) -> Result<Json<RuntimeResponse<()>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    let catalog = catalog_value(&state, &session).await?;
    if !catalog
        .plugins
        .iter()
        .any(|plugin| plugin.source_id == source)
    {
        return Err(RuntimeError::forbidden("当前租户未安装此插件"));
    }
    state
        .store()?
        .set_menu_hidden(&session.tenant_id, &source, hidden)
        .await?;
    Ok(Json(RuntimeResponse { data: () }))
}
