use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use az_plugin_contract::process::ServiceRequest;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

use super::super::{model, services};
use super::model::Gateway;

pub(super) fn router(gateway: Arc<Gateway>) -> Router {
    Router::new()
        .route("/invoke", post(invoke))
        .route("/egress", post(egress))
        .route("/egress/models", get(models))
        .route("/egress/http", post(super::http_egress::request))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .with_state(gateway)
}

pub(super) async fn active(
    gateway: &Gateway,
    headers: &HeaderMap,
) -> Result<Arc<super::super::Components>> {
    let token = headers
        .get("x-aio-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    ensure!(
        Sha256::digest(token.as_bytes()) == Sha256::digest(gateway.token.as_bytes()),
        "process 票据无效"
    );
    let components = gateway.components.upgrade().context("宿主已停止")?;
    let local = if gateway.start.tenant == "development" {
        components
            .development
            .read()
            .await
            .get(&gateway.start.source)
            .map(|instance| instance.backend_digest == gateway.start.revision)
    } else {
        None
    };
    let enabled: bool = if let Some(local) = local {
        local
    } else {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM component_installations WHERE source_id=$1 AND tenant_id=$2 AND digest=$3 AND enabled)").bind(gateway.start.source).bind(&gateway.start.tenant).bind(&gateway.start.revision).fetch_one(&components.pool).await?
    };
    ensure!(enabled, "process 活动版本已撤销");
    Ok(components)
}

async fn invoke(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Json(request): Json<ServiceRequest>,
) -> Response {
    match invoke_inner(&gateway, &headers, request).await {
        Ok(response) => Json(response).into_response(),
        Err(_) => (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error":"资料不可用或调用未授权"})),
        )
            .into_response(),
    }
}

async fn invoke_inner(
    gateway: &Gateway,
    headers: &HeaderMap,
    request: ServiceRequest,
) -> Result<serde_json::Value> {
    let _permit = gateway
        .quota
        .try_acquire()
        .context("process 调用并发已满")?;
    let components = active(gateway, headers).await?;
    ensure!(
        request.tenant_id == gateway.start.tenant && gateway.services.contains(&request.target),
        "跨插件调用未授权"
    );
    let actor: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM component_process_actors WHERE source_id=$1 AND tenant_id=$2 AND user_id=$3)").bind(gateway.start.source).bind(&request.tenant_id).bind(&request.user_id).fetch_one(&components.pool).await?;
    ensure!(
        actor
            && components
                .identity
                .member_active(&request.tenant_id, &request.user_id)
                .await?,
        "process 用户已撤权"
    );
    let local_target = if request.tenant_id == "development" {
        components
            .development
            .read()
            .await
            .iter()
            .find(|(_, instance)| instance.repository == request.target)
            .map(|(source, _)| *source)
    } else {
        None
    };
    let source: Uuid = if let Some(source) = local_target {
        source
    } else {
        sqlx::query_scalar("SELECT s.id FROM component_sources s JOIN component_installations i ON i.source_id=s.id WHERE s.git=$1 AND i.tenant_id=$2 AND i.enabled").bind(&request.target).bind(&request.tenant_id).fetch_optional(&components.pool).await?.context("目标插件未启用")?
    };
    let bundle = components.bundle(source, &request.tenant_id).await?;
    ensure!(
        bundle.manifest().plugin.runtime.process.is_none(),
        "当前 broker 仅开放 Component 服务"
    );
    let background;
    let context = if request.interactive {
        let id = request
            .context_id
            .as_deref()
            .context("交互调用缺少宿主上下文")?;
        let context = components
            .services
            .interactive(id, &request.tenant_id, &request.user_id)?;
        let live = components
            .identity
            .session_active(
                context.session_id.as_deref().context("交互会话缺失")?,
                &request.tenant_id,
                &request.user_id,
            )
            .await?;
        ensure!(live, "交互会话已失效");
        context
    } else {
        let permissions = bundle
            .manifest()
            .plugin
            .permissions
            .iter()
            .map(|p| services::permission(source, p))
            .collect();
        background = components.services.background(
            &request.tenant_id,
            &request.user_id,
            &gateway.start.source.to_string(),
            permissions,
        )?;
        background.context.clone()
    };
    let path = reqwest::Url::parse(&format!("http://plugin{}", request.path))?;
    ensure!(
        path.host_str() == Some("plugin") && path.fragment().is_none(),
        "跨插件路径无效"
    );
    let input = model::Request {
        method: request.method,
        path: path.path().into(),
        query: path.query().map(str::to_owned),
        headers: vec![model::Header {
            name: "content-type".into(),
            value: "application/json".into(),
        }],
        body: serde_json::to_vec(&request.body)?,
    };
    let response = components
        .handle(
            source,
            &request.tenant_id,
            bundle.digest(),
            input.try_into()?,
            context,
        )
        .await?;
    let body: serde_json::Value = if response.body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&response.body)?
    };
    Ok(serde_json::json!({"status":response.status,"body":body}))
}

async fn egress(State(gateway): State<Arc<Gateway>>, headers: HeaderMap, body: Bytes) -> Response {
    match egress_inner(gateway, headers, Some(body)).await {
        Ok(response) => response,
        Err(_) => (StatusCode::BAD_GATEWAY, "模型出站不可用或未授权").into_response(),
    }
}

async fn models(State(gateway): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    match egress_inner(gateway, headers, None).await {
        Ok(response) => response,
        Err(_) => (StatusCode::BAD_GATEWAY, "模型列表不可用或未授权").into_response(),
    }
}

async fn egress_inner(
    gateway: Arc<Gateway>,
    headers: HeaderMap,
    body: Option<Bytes>,
) -> Result<Response> {
    let permit = gateway.quota.clone().try_acquire_owned()?;
    active(&gateway, &headers).await?;
    let endpoint = headers
        .get("x-aio-endpoint")
        .and_then(|v| v.to_str().ok())
        .context("模型地址缺失")?;
    ensure!(
        gateway.endpoints.iter().any(|allowed| endpoint == allowed),
        "模型地址未授权"
    );
    let content_type = if body.is_some() {
        "text/event-stream"
    } else {
        "application/json"
    };
    let mut request = if let Some(body) = body {
        let payload: serde_json::Value = serde_json::from_slice(&body)?;
        ensure!(
            payload["stream"] == true
                && payload["model"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty() && s.len() <= 256),
            "模型请求无效"
        );
        gateway
            .client
            .post(format!("{endpoint}/chat/completions"))
            .header("content-type", "application/json")
            .body(body)
    } else {
        gateway.client.get(format!("{endpoint}/models"))
    };
    if let Some(authorization) = headers.get("authorization") {
        request = request.header("authorization", authorization);
    }
    let response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
        return Ok((status, "模型服务拒绝请求").into_response());
    }
    ensure!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with(content_type)),
        "模型响应类型无效"
    );
    let stream = response
        .bytes_stream()
        .scan((0usize, permit), |(bytes, _), chunk| {
            let item = chunk.map_err(std::io::Error::other).and_then(|chunk| {
                *bytes += chunk.len();
                if *bytes > 2 * 1024 * 1024 {
                    Err(std::io::Error::other("模型响应超过配额"))
                } else {
                    Ok(chunk)
                }
            });
            std::future::ready(Some(item))
        });
    Ok(Response::builder()
        .status(status)
        .header("content-type", content_type)
        .body(Body::from_stream(stream))?)
}
