use std::time::Duration;

use anyhow::Context;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode},
    response::Response,
};
use az_plugin_contract::public::{MAX_PUBLIC_BYTES, PUBLIC_TIMEOUT_SECONDS};
use tokio::time::Instant;
use uuid::Uuid;

use super::super::{RuntimeState, http_error::RuntimeError, request_context::authenticate};
use super::{
    model,
    streaming::{Access, Transfer},
};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DownloadQuery {
    path: String,
    name: String,
    query: Option<String>,
}

pub(super) async fn read(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(token): Path<String>,
    Query(query): Query<DownloadQuery>,
    method: Method,
) -> Result<Response, RuntimeError> {
    let deadline = Instant::now() + Duration::from_secs(PUBLIC_TIMEOUT_SECONDS);
    tokio::time::timeout_at(
        deadline,
        prepare(state, headers, token, query, method, deadline),
    )
    .await
    .context("下载请求超时")?
}

async fn prepare(
    state: RuntimeState,
    headers: HeaderMap,
    token: String,
    query: DownloadQuery,
    method: Method,
    deadline: Instant,
) -> Result<Response, RuntimeError> {
    if query.name.is_empty()
        || query.name.len() > 255
        || query
            .name
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\'))
    {
        return Err(RuntimeError::bad_request("下载文件名无效"));
    }
    if !az_plugin_contract::public::valid_path(&query.path)
        || query
            .query
            .as_ref()
            .is_some_and(|value| value.contains('#'))
    {
        return Err(RuntimeError::bad_request("下载资源路径无效"));
    }
    let session = authenticate(&state, &headers).await?;
    let permit = state.frontend.request_slot()?;
    let grant = super::frontend::validate(&state, &session, &token).await?;
    let components = state.components()?;
    let authorization = components.services.enter(&session, &grant.source_id)?;
    let request = model::Request {
        method: method.to_string(),
        path: query.path,
        query: query.query,
        headers: vec![],
        body: vec![],
    };
    let response = components
        .handle_stream(
            Uuid::parse_str(&grant.source_id)?,
            &session.tenant_id,
            &grant.revision,
            request.try_into()?,
            authorization.context.clone(),
        )
        .await?;
    let tasks = state.plugin_tasks::<super::super::plugins::ComponentsPlugin>()?;
    let access = Access::Session {
        state,
        headers,
        token,
        _authorization: Box::new(authorization),
    };
    access.check().await?;
    if response.status == 404 {
        return Err(RuntimeError::not_found("文件不存在"));
    }
    if response.status != 200 || response.headers.len() > 16 {
        return Err(RuntimeError::bad_request("插件未返回可下载资源"));
    }
    let lengths = response
        .headers
        .iter()
        .filter(|header| header.name.eq_ignore_ascii_case("content-length"))
        .collect::<Vec<_>>();
    if lengths.len() > 1 {
        return Err(RuntimeError::bad_request("响应长度头重复"));
    }
    let expected: Option<u64> = lengths
        .first()
        .map(|header| header.value.parse())
        .transpose()?;
    if expected.is_some_and(|length| length > MAX_PUBLIC_BYTES) {
        return Err(RuntimeError::bad_request("下载超过大小限制"));
    }
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/octet-stream")
        .header("cache-control", "private, no-store")
        .header("x-content-type-options", "nosniff")
        .header("referrer-policy", "no-referrer")
        .header(
            "content-disposition",
            format!(
                "attachment; filename*=UTF-8''{}",
                urlencoding::encode(&query.name)
            ),
        );
    if let Some(length) = expected {
        builder = builder.header("content-length", length);
    }
    if method == Method::HEAD {
        return Ok(builder.body(Body::empty())?);
    }
    let transfer = Transfer {
        access,
        tasks,
        limit: MAX_PUBLIC_BYTES,
        body: response.body,
        bytes: 0,
        expected,
        deadline,
        next_check: Instant::now(),
        _permit: permit,
    };
    Ok(builder.body(transfer.body()?)?)
}
