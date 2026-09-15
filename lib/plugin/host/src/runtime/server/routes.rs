use anyhow::Context as _;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, OriginalUri, Path, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{any, delete, get, post},
};

use super::{
    RuntimeState, frontend_routes,
    http_error::RuntimeError,
    lifecycle::{
        cleanup_new_process, cleanup_new_wasm, deactivate_bound_wasm, deactivate_previous_wasm,
        prepare_bound_wasm, prepare_process_revision, restore_process_binding,
        stop_previous_process,
    },
    management::{create_publish_credential, revoke_publish_credential},
    marketplace_store::PUBLISHED_REGISTRY_SOURCE,
    publication::{authorize_upload, download_package, publish},
    request_context::{
        authenticate, authenticate_manager, authenticate_publisher, authorize_publish_target,
        catalog_for, catalog_value, permitted,
    },
    service_dispatch::{ServiceCall, dispatch},
};
use crate::runtime::{
    InstallPluginRequest, MarketplaceEntry, PageActionRequest, PageActionResult, PageBody,
    PageDefinition, PluginRequest, PublishedPluginView, RuntimeCatalog, RuntimeResponse,
};

pub fn router(state: RuntimeState) -> Router {
    Router::new()
        .route(
            "/api/runtime/{*unknown}",
            any(|| async { StatusCode::NOT_FOUND }),
        )
        .route("/api/runtime/frontend/mount", post(frontend_routes::mount))
        .route(
            "/api/runtime/frontend/{token}/renew",
            post(frontend_routes::renew),
        )
        .route(
            "/api/runtime/frontend/assets/{token}/{*path}",
            get(frontend_routes::asset).layer(super::frontend_delivery::compression()),
        )
        .route(
            "/api/runtime/frontend/{token}/request",
            post(frontend_routes::request),
        )
        .route(
            "/api/runtime/frontend/{token}",
            delete(frontend_routes::unmount),
        )
        .route("/api/runtime/catalog", get(catalog))
        .route("/api/runtime/pages/action", post(page_action))
        .route("/api/runtime/marketplace", get(marketplace))
        .route("/api/runtime/plugins/install", post(install))
        .route("/api/runtime/publish-jobs/{job_id}", get(publish_job))
        .route(
            "/api/runtime/plugins/publish",
            post(publish)
                .layer(DefaultBodyLimit::max(az_plugin_package::MAX_PACKAGE_BYTES))
                .layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    authorize_upload,
                )),
        )
        .route("/api/runtime/packages/{revision}", get(download_package))
        .route("/api/runtime/releases", get(super::releases::list))
        .route(
            "/api/runtime/publish-credentials",
            post(create_publish_credential),
        )
        .route(
            "/api/runtime/publish-credentials/{credential_id}",
            delete(revoke_publish_credential),
        )
        .route(
            "/api/runtime/plugins/{source_id}/events",
            get(lifecycle_events),
        )
        .route("/api/runtime/services/{source_id}/{*path}", any(service))
        .route("/api/runtime/plugins/{source_id}/enable", post(enable))
        .route("/api/runtime/plugins/{source_id}/disable", post(disable))
        .route("/api/runtime/plugins/{source_id}/rollback", post(rollback))
        .route(
            "/api/runtime/plugins/{source_id}/uninstall",
            post(uninstall),
        )
        .merge(super::delivery::router())
        .merge(super::components::router(state.clone()))
        .merge(super::bootstrap::router())
        .merge(super::tools::router())
        .with_state(state)
}

async fn catalog(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<Json<RuntimeResponse<RuntimeCatalog>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    catalog_for(&state, &session).await
}

async fn page_action(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<PageActionRequest>,
) -> Result<Json<RuntimeResponse<PageActionResult>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    let binding = state
        .store
        .active_page_binding(&session.tenant_id, &request.page_id)
        .await?
        .ok_or_else(|| RuntimeError::not_found("当前租户没有提供该页面的活动插件"))?;
    ensure_page_action_allowed(&binding.page, &request.action_id, &session.permissions)?;
    let event = serde_json::to_string(&PluginRequest::PageAction {
        page_id: request.page_id.clone(),
        action_id: request.action_id.clone(),
        tenant_id: session.tenant_id.clone(),
        user_id: session.user_id.clone(),
        body: binding.page.body.clone(),
    })
    .context("序列化页面动作请求失败")?;
    let result = match binding.service.runtime {
        crate::runtime::PluginRuntime::WasmComponent => {
            let manager = state.wasm.clone();
            let tenant_id = session.tenant_id.clone();
            let source_id = binding.source_id.clone();
            let revision = binding.service.revision.clone();
            let output = tokio::task::spawn_blocking(move || {
                manager.handle(&tenant_id, &source_id, &revision, event)
            })
            .await
            .map_err(|error| {
                RuntimeError::bad_request(format!("等待 Wasm 页面动作失败: {error}"))
            })??;
            if !(200..300).contains(&output.status) {
                return Err(RuntimeError::bad_request(format!(
                    "Wasm 页面动作返回 HTTP {}",
                    output.status
                )));
            }
            serde_json::from_str::<PageActionResult>(&output.body)
                .context("解析 Wasm 页面动作结果失败")?
        }
        crate::runtime::PluginRuntime::Process => {
            let endpoint = binding
                .service
                .endpoint
                .as_deref()
                .context("process 页面插件缺少活动 endpoint")?;
            let output = state
                .process
                .request(
                    endpoint,
                    "POST",
                    "/aio/action",
                    None,
                    event.into_bytes(),
                    Some("application/json"),
                    &session.tenant_id,
                    &session.user_id,
                )
                .await?;
            if !output.status.is_success() {
                return Err(RuntimeError::bad_request(format!(
                    "process 页面动作返回 HTTP {}",
                    output.status
                )));
            }
            serde_json::from_slice::<PageActionResult>(&output.body)
                .context("解析 process 页面动作结果失败")?
        }
        _ => return Err(RuntimeError::bad_request("当前页面运行时不支持动作")),
    };
    validate_page_action_result(&binding.page, &result)?;
    state
        .store
        .save_page_state(
            &session.tenant_id,
            &binding.source_id,
            &binding.revision_id,
            &request.page_id,
            binding.state_generation,
            &result.body,
        )
        .await?;
    Ok(Json(RuntimeResponse { data: result }))
}

fn ensure_page_action_allowed(
    page: &PageDefinition,
    action_id: &str,
    permissions: &[String],
) -> Result<(), RuntimeError> {
    if !permitted(page.required_permission.as_deref(), permissions) {
        return Err(RuntimeError::forbidden("当前角色没有页面动作权限"));
    }
    let PageBody::Actions { actions, .. } = &page.body else {
        return Err(RuntimeError::bad_request("当前页面没有运行时动作"));
    };
    if !actions.iter().any(|action| action.id == action_id) {
        return Err(RuntimeError::bad_request("页面动作未声明"));
    }
    Ok(())
}

fn validate_page_action_result(
    page: &PageDefinition,
    result: &PageActionResult,
) -> Result<(), RuntimeError> {
    let PageBody::Actions {
        actions: declared, ..
    } = &page.body
    else {
        return Err(RuntimeError::bad_request("当前页面没有运行时动作"));
    };
    let PageBody::Actions {
        actions: returned, ..
    } = &result.body
    else {
        return Err(RuntimeError::bad_request(
            "页面动作结果必须保持 actions 页面体",
        ));
    };
    if declared != returned {
        return Err(RuntimeError::bad_request(
            "页面动作结果不能修改已发布的动作声明",
        ));
    }
    let mut updated = page.clone();
    updated.body = result.body.clone();
    az_plugin_manifest::validate_page_definitions(&[updated])?;
    Ok(())
}

async fn install(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<InstallPluginRequest>,
) -> Result<Json<RuntimeResponse<RuntimeCatalog>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    super::installation::install(&state, &session.tenant_id, &request).await?;
    catalog_for(&state, &session).await
}

async fn publish_job(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<Json<RuntimeResponse<PublishedPluginView>>, RuntimeError> {
    let publisher = authenticate_publisher(&state, &headers).await?;
    let job = state
        .store
        .publish_job(&publisher.tenant_id, &job_id)
        .await?
        .ok_or_else(|| RuntimeError::not_found("发布任务不存在"))?;
    authorize_publish_target(publisher, &job.git, None)?;
    Ok(Json(RuntimeResponse {
        data: PublishedPluginView {
            job_id: job.id,
            tenant_id: job.tenant_id,
            source_id: job.source_id,
            revision: job.revision,
            runtime: job.runtime,
            page_count: job.page_count,
            state: job.state,
            detail: job.detail,
        },
    }))
}

async fn enable(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(source_id): Path<String>,
) -> Result<Json<RuntimeResponse<RuntimeCatalog>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    if let Some(components) = &state.components
        && components.contains_source(&source_id).await?
    {
        components
            .change(
                &session.tenant_id,
                uuid::Uuid::parse_str(&source_id).context("来源无效")?,
                "enable",
            )
            .await?;
        return catalog_for(&state, &session).await;
    }
    state
        .store
        .bound_runtime(&session.tenant_id, &source_id)
        .await?;
    let git: Option<String> = sqlx::query_scalar("SELECT git FROM plugin_sources WHERE id=$1")
        .bind(&source_id)
        .fetch_optional(&state.store.pool)
        .await
        .map_err(anyhow::Error::from)?;
    if let Some(git) = git
        && let Some(latest) = state.store.published_marketplace_entry(&git, None).await?
    {
        super::installation::install(
            &state,
            &session.tenant_id,
            &InstallPluginRequest {
                git,
                rev: Some(latest.rev),
            },
        )
        .await?;
        return catalog_for(&state, &session).await;
    }
    let lifecycle_lock = state.activation_lock(&session.tenant_id, &source_id)?;
    let _lifecycle_guard = lifecycle_lock.lock().await;
    let target = state
        .store
        .bound_runtime(&session.tenant_id, &source_id)
        .await?;
    let instance = if target.runtime == crate::runtime::PluginRuntime::Process {
        Some(
            prepare_process_revision(&state, &session.tenant_id, &source_id, &target.revision)
                .await?
                .0,
        )
    } else {
        None
    };
    let wasm = if target.runtime == crate::runtime::PluginRuntime::WasmComponent {
        Some(prepare_bound_wasm(&state, &session.tenant_id, &source_id, &target).await?)
    } else {
        None
    };
    if let Err(error) = state
        .store
        .set_enabled(&session.tenant_id, &source_id, true, instance.as_ref())
        .await
    {
        let _ = cleanup_new_process(&state, instance.as_ref()).await;
        let _ = cleanup_new_wasm(
            &state,
            &session.tenant_id,
            &source_id,
            &target.revision,
            wasm.as_ref(),
        );
        return Err(error.into());
    }
    catalog_for(&state, &session).await
}

async fn disable(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(source_id): Path<String>,
) -> Result<Json<RuntimeResponse<RuntimeCatalog>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    if let Some(components) = &state.components {
        if components.contains_source(&source_id).await? {
            components
                .change(
                    &session.tenant_id,
                    uuid::Uuid::parse_str(&source_id).context("来源无效")?,
                    "disable",
                )
                .await?;
            return catalog_for(&state, &session).await;
        }
        if let Ok(source) = uuid::Uuid::parse_str(&source_id) {
            components
                .require_no_children(&session.tenant_id, source)
                .await?;
        }
    }
    let lifecycle_lock = state.activation_lock(&session.tenant_id, &source_id)?;
    let _lifecycle_guard = lifecycle_lock.lock().await;
    let previous = state
        .store
        .active_process(&session.tenant_id, &source_id)
        .await?;
    let target = state
        .store
        .bound_runtime(&session.tenant_id, &source_id)
        .await?;
    if let Err(error) = stop_previous_process(&state, previous, None).await {
        return Err(error.into());
    }
    if let Err(error) = state
        .store
        .set_enabled(&session.tenant_id, &source_id, false, None)
        .await
    {
        if let Err(recovery) =
            restore_process_binding(&state, &session.tenant_id, &source_id, Some(&target)).await
        {
            return Err(anyhow::anyhow!(
                "停用插件失败: {error:#}; 恢复旧 process 插件失败: {recovery:#}"
            )
            .into());
        }
        return Err(error.into());
    }
    deactivate_bound_wasm(&state, &session.tenant_id, &source_id, &target)?;
    catalog_for(&state, &session).await
}

async fn uninstall(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(source_id): Path<String>,
) -> Result<Json<RuntimeResponse<RuntimeCatalog>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    if let Some(components) = &state.components {
        if components.contains_source(&source_id).await? {
            components
                .change(
                    &session.tenant_id,
                    uuid::Uuid::parse_str(&source_id).context("来源无效")?,
                    "uninstall",
                )
                .await?;
            return catalog_for(&state, &session).await;
        }
        if let Ok(source) = uuid::Uuid::parse_str(&source_id) {
            components
                .require_no_children(&session.tenant_id, source)
                .await?;
        }
    }
    let lifecycle_lock = state.activation_lock(&session.tenant_id, &source_id)?;
    let _lifecycle_guard = lifecycle_lock.lock().await;
    let previous = state
        .store
        .active_process(&session.tenant_id, &source_id)
        .await?;
    let target = state
        .store
        .bound_runtime(&session.tenant_id, &source_id)
        .await?;
    if let Err(error) = stop_previous_process(&state, previous, None).await {
        return Err(error.into());
    }
    if let Err(error) = state.store.uninstall(&session.tenant_id, &source_id).await {
        if let Err(recovery) =
            restore_process_binding(&state, &session.tenant_id, &source_id, Some(&target)).await
        {
            return Err(anyhow::anyhow!(
                "卸载插件失败: {error:#}; 恢复旧 process 插件失败: {recovery:#}"
            )
            .into());
        }
        return Err(error.into());
    }
    deactivate_bound_wasm(&state, &session.tenant_id, &source_id, &target)?;
    catalog_for(&state, &session).await
}

async fn rollback(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(source_id): Path<String>,
) -> Result<Json<RuntimeResponse<RuntimeCatalog>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    if let Some(components) = &state.components
        && components.contains_source(&source_id).await?
    {
        components
            .change(
                &session.tenant_id,
                uuid::Uuid::parse_str(&source_id).context("来源无效")?,
                "rollback",
            )
            .await?;
        return catalog_for(&state, &session).await;
    }
    let lifecycle_lock = state.activation_lock(&session.tenant_id, &source_id)?;
    let _lifecycle_guard = lifecycle_lock.lock().await;
    let previous = state
        .store
        .active_process(&session.tenant_id, &source_id)
        .await?;
    let previous_target = if previous.is_some() {
        Some(
            state
                .store
                .bound_runtime(&session.tenant_id, &source_id)
                .await?,
        )
    } else {
        None
    };
    let previous_wasm = state
        .store
        .active_wasm_revision(&session.tenant_id, &source_id)
        .await?;
    let target = state
        .store
        .rollback_target(&session.tenant_id, &source_id)
        .await?;
    let instance = if target.runtime == crate::runtime::PluginRuntime::Process {
        Some(
            prepare_process_revision(&state, &session.tenant_id, &source_id, &target.revision)
                .await?
                .0,
        )
    } else {
        None
    };
    let wasm = if target.runtime == crate::runtime::PluginRuntime::WasmComponent {
        Some(prepare_bound_wasm(&state, &session.tenant_id, &source_id, &target).await?)
    } else {
        None
    };
    if let Err(error) = stop_previous_process(&state, previous, instance.as_ref()).await {
        let _ = cleanup_new_process(&state, instance.as_ref()).await;
        let _ = cleanup_new_wasm(
            &state,
            &session.tenant_id,
            &source_id,
            &target.revision,
            wasm.as_ref(),
        );
        return Err(error.into());
    }
    if let Err(error) = state
        .store
        .rollback_to(&session.tenant_id, &source_id, &target, instance.as_ref())
        .await
    {
        let _ = cleanup_new_process(&state, instance.as_ref()).await;
        let _ = cleanup_new_wasm(
            &state,
            &session.tenant_id,
            &source_id,
            &target.revision,
            wasm.as_ref(),
        );
        if let Err(recovery) = restore_process_binding(
            &state,
            &session.tenant_id,
            &source_id,
            previous_target.as_ref(),
        )
        .await
        {
            return Err(anyhow::anyhow!(
                "回滚插件失败: {error:#}; 恢复旧 process 插件失败: {recovery:#}"
            )
            .into());
        }
        return Err(error.into());
    }
    deactivate_previous_wasm(
        &state,
        &session.tenant_id,
        &source_id,
        previous_wasm.as_deref(),
        wasm.as_ref().map(|_| target.revision.as_str()),
    )?;
    catalog_for(&state, &session).await
}

async fn marketplace(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Result<Json<RuntimeResponse<Vec<super::tools::MarketplaceItem>>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    let catalog = catalog_value(&state, &session).await?;
    let mut entries = if let Some(components) = &state.components {
        components.entries(&session.tenant_id).await?
    } else {
        Vec::new()
    };
    {
        for mut entry in state
            .store
            .marketplace_entries(PUBLISHED_REGISTRY_SOURCE)
            .await?
        {
            if let Some(plugin) = catalog
                .plugins
                .iter()
                .find(|plugin| plugin.git == entry.git)
            {
                enrich_marketplace_entry(&mut entry, plugin);
            }
            if !entries
                .iter()
                .any(|current: &MarketplaceEntry| current.git == entry.git)
            {
                entries.push(entry);
            }
        }
    }
    for plugin in &catalog.plugins {
        if !entries
            .iter()
            .any(|entry: &MarketplaceEntry| entry.git == plugin.git)
        {
            entries.push(unlisted_entry(plugin));
        }
    }
    let mut items = entries
        .into_iter()
        .map(super::tools::MarketplaceItem::Plugin)
        .collect::<Vec<_>>();
    items.extend(super::tools::entries(&state.store.pool).await?);
    Ok(Json(RuntimeResponse { data: items }))
}

fn enrich_marketplace_entry(
    entry: &mut MarketplaceEntry,
    plugin: &crate::runtime::InstalledPluginView,
) {
    entry.source_id = Some(plugin.source_id.clone());
    entry.installed = true;
    entry.state = Some(plugin.state);
    entry.active_revision = Some(plugin.revision.clone());
    entry.runtime = Some(plugin.runtime);
    entry.capabilities.clone_from(&plugin.capabilities);
}

async fn lifecycle_events(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(source_id): Path<String>,
) -> Result<Json<RuntimeResponse<Vec<crate::runtime::PluginLifecycleEvent>>>, RuntimeError> {
    let session = authenticate_manager(&state, &headers).await?;
    let events = state
        .store
        .lifecycle_events(&session.tenant_id, &source_id)
        .await?;
    Ok(Json(RuntimeResponse { data: events }))
}

fn unlisted_entry(plugin: &crate::runtime::InstalledPluginView) -> MarketplaceEntry {
    let title = plugin
        .git
        .trim_end_matches(".git")
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("Git plugin")
        .to_owned();
    MarketplaceEntry {
        parent_git: None,
        parent_title: None,
        git: plugin.git.clone(),
        rev: plugin.revision.clone(),
        title,
        summary: "此插件已安装在当前工作区，未收录于官方市场。".to_owned(),
        license: "未收录".to_owned(),
        tags: vec!["unlisted".to_owned()],
        installed: true,
        source_id: Some(plugin.source_id.clone()),
        state: Some(plugin.state),
        active_revision: Some(plugin.revision.clone()),
        runtime: Some(plugin.runtime),
        capabilities: plugin.capabilities.clone(),
    }
}

async fn service(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path((source_id, path)): Path<(String, String)>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    body: Bytes,
) -> Result<Response, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    let lock = state.activation_lock(&session.tenant_id, &source_id)?;
    let _guard = lock.lock().await;
    let binding = state
        .store
        .active_service(&session.tenant_id, &source_id)
        .await?
        .ok_or_else(|| RuntimeError::not_found("当前租户没有活动的服务插件"))?;
    let path = format!("/{path}");
    let output = dispatch(
        &state,
        &session,
        &source_id,
        &binding,
        ServiceCall {
            method: method.as_str(),
            path: &path,
            query: uri.query(),
            body: body.to_vec(),
            content_type: headers
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
        },
    )
    .await?;
    response(output.status, output.content_type.as_deref(), output.body)
}

fn response(
    status: StatusCode,
    content_type: Option<&str>,
    body: Vec<u8>,
) -> Result<Response, RuntimeError> {
    let mut response = (status, body).into_response();
    if let Some(content_type) = content_type {
        let content_type = HeaderValue::from_str(content_type)
            .map_err(|_| RuntimeError::bad_request("插件返回了无效 Content-Type"))?;
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, content_type);
    }
    Ok(response)
}

#[cfg(test)]
#[path = "routes_tests.rs"]
mod tests;
