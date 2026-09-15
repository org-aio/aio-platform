use super::super::{
    RuntimeState,
    frontend_access::FrontendGrant,
    frontend_model::MountResponse,
    http_error::RuntimeError,
    request_context::{authenticate, permitted, session_context, tenant_context},
};
use super::model;
use crate::identity::SessionContext;
use crate::runtime::RuntimeResponse;
use anyhow::{Context, Result, ensure};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use kuchikiki::traits::TendrilSink;
use std::collections::BTreeMap;
use std::time::Instant;
use uuid::Uuid;

fn identity(page: &str) -> Result<(Uuid, &str)> {
    let (source, page) = page
        .strip_prefix("component:")
        .and_then(|id| id.split_once(':'))
        .context("Component 页面 ID 无效")?;
    Ok((Uuid::parse_str(source)?, page))
}

async fn page(
    state: &RuntimeState,
    session: &SessionContext,
    id: &str,
) -> Result<(Uuid, String, String, model::Page)> {
    let (source, id) = identity(id)?;
    let (digest, generation, description) = state
        .components()?
        .description(&session.tenant_id, source)
        .await?;
    let page = description
        .pages
        .into_iter()
        .find(|p| p.id == id)
        .context("插件未声明该页面")?;
    let permission = page
        .permission
        .as_deref()
        .map(|p| super::services::permission(source, p));
    ensure!(
        permitted(permission.as_deref(), &session.permissions),
        "当前用户无页面访问权限"
    );
    Ok((source, digest, generation, page))
}

pub(in crate::runtime::server) async fn mount(
    state: &RuntimeState,
    headers: &HeaderMap,
    session: &SessionContext,
    id: &str,
) -> Result<MountResponse> {
    let _permit = state.frontend.request_slot()?;
    let (source, revision, generation, page) = page(state, session, id).await?;
    let bundle = state
        .components()?
        .bundle(source, &session.tenant_id)
        .await?;
    ensure!(bundle.digest() == revision, "插件安装版本发生变化");
    let entry = page.entry;
    let assets = bundle
        .frontend_assets()
        .iter()
        .filter(|(path, _)| **path != entry)
        .map(|(path, asset)| (path.to_owned(), asset.digest.clone()))
        .collect::<BTreeMap<_, _>>();
    let asset_sizes = bundle
        .frontend_assets()
        .iter()
        .filter(|(path, _)| **path != entry)
        .map(|(path, asset)| (path.to_owned(), asset.size))
        .collect();
    let token = state.frontend.issue(FrontendGrant {
        cookie: headers
            .get(header::COOKIE)
            .cloned()
            .context("会话 Cookie 缺失")?,
        session_id: session.session_id.clone(),
        tenant_id: session.tenant_id.clone(),
        user_id: session.user_id.clone(),
        page_id: id.into(),
        source_id: source.to_string(),
        activation_generation: generation.clone(),
        revision: revision.clone(),
        entry: entry.clone(),
        frontend_path: bundle.manifest().plugin.frontend.path.clone(),
        assets: assets.clone(),
        issued: Instant::now(),
    })?;
    Ok(MountResponse {
        development: state.config.development.is_some(),
        src: format!("/api/runtime/components/assets/{token}/{entry}"),
        token,
        revision,
        generation,
        session_context: session_context(session),
        context: tenant_context(session)?,
        assets,
        asset_sizes,
        abi: Some(2),
    })
}

async fn validate(
    state: &RuntimeState,
    session: &SessionContext,
    token: &str,
) -> Result<FrontendGrant, RuntimeError> {
    let grant = state
        .frontend
        .get(token)
        .map_err(|_| RuntimeError::unauthorized("挂载已过期"))?;
    if grant.session_id != session.session_id
        || grant.tenant_id != session.tenant_id
        || grant.user_id != session.user_id
    {
        return Err(RuntimeError::forbidden("挂载不属于当前登录或租户"));
    }
    let (source, digest, generation, page) = page(state, session, &grant.page_id).await?;
    if source.to_string() != grant.source_id
        || digest != grant.revision
        || generation != grant.activation_generation
        || page.entry != grant.entry
    {
        return Err(RuntimeError::forbidden("插件版本已变化，请重新挂载"));
    }
    Ok(grant)
}

pub(super) async fn asset(
    State(state): State<RuntimeState>,
    Path((token, path)): Path<(String, String)>,
) -> Result<Response, RuntimeError> {
    az_plugin_bundle::validate_relative_path(&path)?;
    let _permit = state.frontend.request_slot()?;
    let grant = state
        .frontend
        .get(&token)
        .map_err(|_| RuntimeError::unauthorized("挂载已过期"))?;
    let mut cookie = HeaderMap::new();
    cookie.insert(header::COOKIE, grant.cookie.clone());
    let session = authenticate(&state, &cookie).await?;
    let grant = validate(&state, &session, &token).await?;
    let bundle = state
        .components()?
        .bundle(
            Uuid::parse_str(&grant.source_id).context("来源无效")?,
            &session.tenant_id,
        )
        .await?;
    if bundle.digest() != grant.revision {
        return Err(RuntimeError::forbidden("插件版本已撤销"));
    }
    let bytes = if path == super::super::frontend_document::MODULES_PATH {
        super::super::frontend_document::MODULES
    } else {
        bundle
            .frontend(&path)
            .ok_or_else(|| RuntimeError::not_found("资产不属于当前插件包"))?
    };
    let prefix = format!(
        "{}/api/runtime/components/assets/{token}/",
        state.frontend.origin
    );
    let bytes = if path == grant.entry {
        render(
            bytes,
            &prefix,
            &path,
            &token,
            state.config.development.is_some(),
        )?
    } else {
        bytes.to_vec()
    };
    let mut response = bytes.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(
            mime_guess::from_path(&path)
                .first_or_octet_stream()
                .as_ref(),
        )?,
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache, no-transform"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CONTENT_SECURITY_POLICY,HeaderValue::from_str(&format!("sandbox allow-scripts allow-forms; default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval' blob: {prefix}; connect-src {prefix} blob:; style-src 'unsafe-inline' blob: {prefix}; img-src data: blob: {prefix}; font-src data: {prefix}; object-src 'none'; frame-src 'none'; worker-src blob:; base-uri {prefix}; form-action 'none'; frame-ancestors 'self'"))?);
    Ok(response)
}

fn render(
    bytes: &[u8],
    prefix: &str,
    entry: &str,
    token: &str,
    development: bool,
) -> Result<Vec<u8>> {
    let document = kuchikiki::parse_html()
        .one(std::str::from_utf8(bytes)?)
        .document_node;
    for script in document
        .select("script[type='module'], script[type='importmap']")
        .map_err(|_| anyhow::anyhow!("解析模块入口失败"))?
    {
        let mut attributes = script.attributes.borrow_mut();
        let kind = if attributes.get("type") == Some("module") {
            "module-shim"
        } else {
            "importmap-shim"
        };
        attributes.insert("type", kind.to_owned());
    }
    for node in document
        .select("base")
        .map_err(|_| anyhow::anyhow!("解析 HTML 失败"))?
    {
        node.as_node().detach();
    }
    let base = kuchikiki::parse_html()
        .one("<head><base></head>")
        .document_node
        .select_first("base")
        .map_err(|_| anyhow::anyhow!("创建 base 失败"))?;
    let directory = entry
        .rsplit_once('/')
        .map(|(dir, _)| format!("{dir}/"))
        .unwrap_or_default();
    base.attributes
        .borrow_mut()
        .insert("href", format!("{prefix}{directory}"));
    let node = base.as_node().clone();
    node.detach();
    let head = document
        .select_first("head")
        .map_err(|_| anyhow::anyhow!("入口缺少 head"))?
        .as_node()
        .clone();
    let script = kuchikiki::parse_html()
        .one("<script></script>")
        .document_node
        .select_first("script")
        .map_err(|_| anyhow::anyhow!("创建 SDK 脚本失败"))?;
    script
        .attributes
        .borrow_mut()
        .insert("data-token", token.to_owned());
    script
        .attributes
        .borrow_mut()
        .insert("data-root", prefix.to_owned());
    script
        .attributes
        .borrow_mut()
        .insert("data-development", development.to_string());
    let script = script.as_node().clone();
    script.detach();
    for source in [
        az_plugin_runtime::FRONTEND_LIFECYCLE,
        az_plugin_runtime::FRONTEND_WASM,
        az_plugin_runtime::FRONTEND_GUEST,
        include_str!("frontend_assets.js"),
    ] {
        script.append(kuchikiki::NodeRef::new_text(source));
    }
    let loader = kuchikiki::parse_html()
        .one("<script></script>")
        .document_node
        .select_first("script")
        .map_err(|_| anyhow::anyhow!("创建模块加载器失败"))?;
    loader.attributes.borrow_mut().insert(
        "src",
        format!("{prefix}{}", super::super::frontend_document::MODULES_PATH),
    );
    let loader = loader.as_node().clone();
    loader.detach();
    head.prepend(loader);
    head.prepend(script);
    head.prepend(node);
    let mut output = Vec::new();
    document.serialize(&mut output)?;
    Ok(output)
}

pub(super) async fn request(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(token): Path<String>,
    Json(request): Json<model::Request>,
) -> Result<Json<RuntimeResponse<model::Response>>, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    let _permit = state.frontend.request_slot()?;
    let grant = validate(&state, &session, &token).await?;
    let components = state.components()?;
    let authorization = components.services.enter(&session)?;
    let response = components
        .handle(
            Uuid::parse_str(&grant.source_id).context("来源无效")?,
            &session.tenant_id,
            &grant.revision,
            request.try_into()?,
            authorization.context.clone(),
        )
        .await?;
    Ok(Json(RuntimeResponse {
        data: response.into(),
    }))
}

pub(super) async fn renew(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Result<StatusCode, RuntimeError> {
    let session = authenticate(&state, &headers).await?;
    validate(&state, &session, &token).await?;
    state.frontend.renew(&token)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_shared_loaders_before_plugin_scripts() -> Result<()> {
        let output = render(
            b"<html><head><script type='module' src='app.mjs'></script></head><body></body></html>",
            "https://aio.test/assets/ticket/",
            "index.html",
            "ticket",
            false,
        )?;
        let document = kuchikiki::parse_html()
            .one(String::from_utf8(output)?)
            .document_node;
        let scripts = document
            .select("script")
            .map_err(|_| anyhow::anyhow!("script selector failed"))?
            .collect::<Vec<_>>();
        assert_eq!(scripts.len(), 3);
        assert_eq!(
            scripts[0].text_contents(),
            format!(
                "{}{}{}{}",
                az_plugin_runtime::FRONTEND_LIFECYCLE,
                az_plugin_runtime::FRONTEND_WASM,
                az_plugin_runtime::FRONTEND_GUEST,
                include_str!("frontend_assets.js")
            )
        );
        assert_eq!(
            scripts[0].attributes.borrow().get("data-root"),
            Some("https://aio.test/assets/ticket/")
        );
        assert_eq!(
            scripts[0].attributes.borrow().get("data-token"),
            Some("ticket")
        );
        assert_eq!(
            scripts[1].attributes.borrow().get("src"),
            Some("https://aio.test/assets/ticket/__aio_modules.js")
        );
        assert_eq!(scripts[2].attributes.borrow().get("src"), Some("app.mjs"));
        assert_eq!(
            scripts[2].attributes.borrow().get("type"),
            Some("module-shim")
        );
        Ok(())
    }
}
