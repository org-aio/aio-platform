use std::time::Instant;

use crate::identity::SessionContext;
use anyhow::Context as _;
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use az_plugin_manifest::{ComponentResponse, validate_frontend_path};
use sha2::{Digest as _, Sha256};

use super::{
    RuntimeState,
    frontend_access::FrontendGrant,
    frontend_document,
    frontend_model::{FrontendRequest, MountRequest, MountResponse},
    http_error::RuntimeError,
    request_context::{authenticate, permitted},
    service_dispatch::{ServiceCall, dispatch},
    store::PageBinding,
};
use crate::runtime::{PageBody, RuntimeResponse};

pub(super) async fn mount(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(request): Json<MountRequest>,
) -> Result<Json<RuntimeResponse<MountResponse>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    if request.page_id.starts_with("component:") {
        return Ok(Json(RuntimeResponse {
            data: super::components::mount(&state, &headers, &session, &request.page_id).await?,
        }));
    }
    let _slot = request_slot(&state)?;
    let binding = page(&state, &session, &request.page_id).await?;
    let source_id = binding.source_id.clone();
    let lock = state.activation_lock(&session.tenant_id, &binding.source_id)?;
    let _guard = lock.lock().await;
    let binding = page(&state, &session, &request.page_id).await?;
    if binding.source_id != source_id {
        return Err(RuntimeError::forbidden("活动页面来源已变化，请重试挂载"));
    }
    let entry = frontend_entry(&binding)?.to_owned();
    let session_context = super::request_context::session_context(&session);
    let context = super::request_context::tenant_context(&session)?;
    let generation = binding.activation_generation.clone();
    let package =
        super::frontend_package::prepare(&state, &binding.service.revision, &entry).await?;
    let grant = FrontendGrant {
        cookie: headers
            .get(header::COOKIE)
            .cloned()
            .context("前端挂载缺少会话 Cookie")?,
        session_id: session.session_id,
        tenant_id: session.tenant_id,
        user_id: session.user_id,
        page_id: request.page_id,
        source_id: binding.source_id,
        activation_generation: binding.activation_generation,
        revision: binding.service.revision.clone(),
        entry: entry.clone(),
        frontend_path: package.path.clone(),
        assets: package.assets.clone(),
        issued: Instant::now(),
    };
    let token = state.frontend.issue(grant)?;
    Ok(Json(RuntimeResponse {
        data: MountResponse {
            development: state.config.development.is_some(),
            abi: None,
            src: format!("/api/runtime/frontend/assets/{token}/{entry}"),
            token,
            revision: binding.service.revision,
            generation,
            session_context,
            context,
            assets: package.assets.clone(),
            asset_sizes: package.asset_sizes.clone(),
        },
    }))
}

pub(super) async fn asset(
    State(state): State<RuntimeState>,
    request_headers: HeaderMap,
    Path((token, path)): Path<(String, String)>,
) -> Result<Response, RuntimeError> {
    if path != frontend_document::BRIDGE_PATH && path != frontend_document::MODULES_PATH {
        validate_frontend_path(&path)?;
    }
    let _slot = request_slot(&state)?;
    let grant = grant(&state, &token)?;
    let lock = state.activation_lock(&grant.tenant_id, &grant.source_id)?;
    let _guard = lock.lock().await;
    let grant = self::grant(&state, &token)?;
    let mut headers = HeaderMap::new();
    headers.insert(header::COOKIE, grant.cookie.clone());
    let session = authenticate(&state, &headers).await?;
    validate_binding(&state, &session, &grant).await?;
    let prefix = format!(
        "{}/api/runtime/frontend/assets/{token}/",
        state.frontend.origin
    );
    let (mut bytes, content_type) = if path == frontend_document::BRIDGE_PATH {
        (
            frontend_document::BRIDGE.as_bytes().to_vec(),
            "application/javascript".to_owned(),
        )
    } else if path == frontend_document::MODULES_PATH {
        (
            frontend_document::MODULES.to_vec(),
            "application/javascript".to_owned(),
        )
    } else {
        let expected = grant
            .assets
            .get(&path)
            .ok_or_else(|| RuntimeError::not_found("插件包未声明该前端文件"))?;
        let artifact = state
            .repository
            .artifact(&grant.revision, &format!("{}/{path}", grant.frontend_path))?;
        let bytes = tokio::fs::read(artifact)
            .await
            .context("读取插件前端资产失败")?;
        if format!("{:x}", Sha256::digest(&bytes)) != *expected {
            return Err(RuntimeError::bad_request("前端资产摘要与已发布版本不一致"));
        }
        (
            bytes,
            mime_guess::from_path(&path)
                .first_or_octet_stream()
                .to_string(),
        )
    };
    if path == grant.entry {
        bytes = frontend_document::render_entry(
            &bytes,
            &prefix,
            &grant.entry,
            &token,
            state.config.development.is_some(),
        )?;
    }
    let etag = (path != grant.entry).then(|| format!("\"{:x}\"", Sha256::digest(&bytes)));
    let not_modified = etag.as_ref().is_some_and(|expected| {
        request_headers
            .get(header::IF_NONE_MATCH)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value.split(',').any(|tag| {
                    let tag = tag.trim().strip_prefix("W/").unwrap_or(tag.trim());
                    tag == expected || tag == "*"
                })
            })
    });
    // 条件请求也必须先通过会话、活动版本和资产摘要校验，缓存不能绕过撤销。
    let mut response = if not_modified {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        bytes.into_response()
    };
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(&content_type)?);
    // 禁止边缘代理向隔离文档注入宿主未授权的脚本。
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if etag.is_some() {
            "private, no-cache, no-transform"
        } else {
            "private, no-store, no-transform"
        }),
    );
    if let Some(etag) = etag {
        headers.insert(header::ETAG, HeaderValue::from_str(&etag)?);
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_str(&frontend_document::content_policy(&prefix))?,
    );
    Ok(response)
}

pub(super) async fn request(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(token): Path<String>,
    Json(request): Json<FrontendRequest>,
) -> Result<Json<RuntimeResponse<ComponentResponse>>, RuntimeError> {
    authenticate(&state, &headers).await?;
    let _slot = request_slot(&state)?;
    let grant = grant(&state, &token)?;
    validate_request(&request)?;
    let lock = state.activation_lock(&grant.tenant_id, &grant.source_id)?;
    let _guard = lock.lock().await;
    let grant = self::grant(&state, &token)?;
    let session = authenticate(&state, &headers).await?;
    let binding = validate_binding(&state, &session, &grant).await?;
    let output = dispatch(
        &state,
        &session,
        &grant.source_id,
        &binding.service,
        ServiceCall {
            method: &request.method,
            path: &request.path,
            query: request.query.as_deref(),
            body: request.body.into_bytes(),
            content_type: Some("application/json"),
        },
    )
    .await?;
    Ok(Json(RuntimeResponse {
        data: ComponentResponse {
            status: output.status.as_u16(),
            content_type: output
                .content_type
                .unwrap_or_else(|| "application/octet-stream".to_owned()),
            body: String::from_utf8(output.body).context("前端通信桥只接受 UTF-8 服务结果")?,
        },
    }))
}

pub(super) async fn unmount(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Result<StatusCode, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    let grant = grant(&state, &token)?;
    if session.session_id != grant.session_id || session.user_id != grant.user_id {
        return Err(RuntimeError::forbidden("不能释放其他会话的前端挂载"));
    }
    state.frontend.remove(&token)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn renew(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Result<StatusCode, RuntimeError> {
    authenticate(&state, &headers).await?;
    let _slot = request_slot(&state)?;
    let grant = grant(&state, &token)?;
    let lock = state.activation_lock(&grant.tenant_id, &grant.source_id)?;
    let _guard = lock.lock().await;
    let grant = self::grant(&state, &token)?;
    let session = authenticate(&state, &headers).await?;
    validate_binding(&state, &session, &grant).await?;
    state.frontend.renew(&token)?;
    Ok(StatusCode::NO_CONTENT)
}

fn grant(state: &RuntimeState, token: &str) -> Result<FrontendGrant, RuntimeError> {
    state
        .frontend
        .get(token)
        .map_err(|_| RuntimeError::unauthorized("前端挂载凭证无效或已过期"))
}

fn request_slot(state: &RuntimeState) -> Result<tokio::sync::OwnedSemaphorePermit, RuntimeError> {
    state
        .frontend
        .request_slot()
        .map_err(|_| RuntimeError::unavailable("前端请求超过宿主并发配额"))
}

async fn page(
    state: &RuntimeState,
    session: &SessionContext,
    page_id: &str,
) -> Result<PageBinding, RuntimeError> {
    let binding = state
        .store
        .active_page_binding(&session.tenant_id, page_id)
        .await?
        .ok_or_else(|| RuntimeError::not_found("当前租户没有该活动页面"))?;
    if !permitted(
        binding.page.required_permission.as_deref(),
        &session.permissions,
    ) {
        return Err(RuntimeError::forbidden("当前用户没有该页面权限"));
    }
    Ok(binding)
}

async fn validate_binding(
    state: &RuntimeState,
    session: &SessionContext,
    grant: &FrontendGrant,
) -> Result<PageBinding, RuntimeError> {
    validate_session(session, grant)?;
    let binding = page(state, session, &grant.page_id).await?;
    if binding.source_id != grant.source_id
        || binding.activation_generation != grant.activation_generation
        || binding.service.revision != grant.revision
        || frontend_entry(&binding)? != grant.entry
    {
        return Err(RuntimeError::forbidden(
            "插件活动版本已变化，请重新打开页面",
        ));
    }
    Ok(binding)
}

fn validate_session(session: &SessionContext, grant: &FrontendGrant) -> Result<(), RuntimeError> {
    if session.session_id != grant.session_id
        || session.user_id != grant.user_id
        || session.tenant_id != grant.tenant_id
    {
        return Err(RuntimeError::forbidden(
            "会话或租户上下文已变化，请重新打开页面",
        ));
    }
    Ok(())
}

fn frontend_entry(binding: &PageBinding) -> Result<&str, RuntimeError> {
    match &binding.page.body {
        PageBody::Frontend { entry } => Ok(entry),
        _ => Err(RuntimeError::bad_request("当前页面没有前端二进制入口")),
    }
}

fn validate_request(request: &FrontendRequest) -> Result<(), RuntimeError> {
    if !matches!(
        request.method.as_str(),
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
    ) {
        return Err(RuntimeError::bad_request("前端请求方法不受支持"));
    }
    let path = request
        .path
        .strip_prefix('/')
        .ok_or_else(|| RuntimeError::bad_request("服务路径必须以 / 开始"))?;
    if path.len() > 2048
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || !path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/_-.~".contains(&byte))
    {
        return Err(RuntimeError::bad_request("服务路径含有非法字符或路径段"));
    }
    if request.body.len() > 1024 * 1024
        || request
            .query
            .as_ref()
            .is_some_and(|query| query.len() > 8192 || query.chars().any(char::is_control))
    {
        return Err(RuntimeError::bad_request(
            "前端请求超过大小限制或包含非法参数",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontend_grants_reject_other_sessions_and_tenants() {
        let grant = super::super::frontend_tests::grant();
        let mut session = super::super::frontend_tests::session();
        assert!(validate_session(&session, &grant).is_ok());
        session.tenant_id = "tenant-b".to_owned();
        assert!(validate_session(&session, &grant).is_err());
        session.tenant_id = grant.tenant_id.clone();
        session.session_id = "session-b".to_owned();
        assert!(validate_session(&session, &grant).is_err());
    }

    #[test]
    fn frontend_requests_cannot_escape_declared_route_paths() {
        let mut request = FrontendRequest {
            method: "POST".to_owned(),
            path: "/counter/increment".to_owned(),
            query: None,
            body: "{}".to_owned(),
        };
        assert!(validate_request(&request).is_ok());
        for path in [
            "https://evil.example",
            "/counter/../private",
            "/counter/%2e%2e/private",
            "//private",
            "/counter?redirect=secret",
            "/counter\\private",
        ] {
            request.path = path.to_owned();
            assert!(validate_request(&request).is_err(), "{path}");
        }
        request.path = "/counter".to_owned();
        request.method = "CONNECT".to_owned();
        assert!(validate_request(&request).is_err());
    }
}
