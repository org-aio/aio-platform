use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use az_plugin_contract::process::{MeterRequest, ServiceRequest};
use az_plugin_contract::{InvocationScope, RequestContext};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

use super::super::{model, services};
use super::model::Gateway;

pub(super) fn router(gateway: Arc<Gateway>) -> Router {
    Router::new()
        .route("/invoke", post(invoke))
        .route("/workers", post(super::workers::invoke))
        .route("/egress", post(egress))
        .route("/egress/responses", post(responses))
        .route("/egress/models", get(models))
        .route("/egress/http", post(super::http_egress::request))
        .route("/meter", post(meter))
        .route("/cryptography/seal", post(seal))
        .route("/cryptography/open", post(open))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .with_state(gateway)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CryptographyRequest {
    purpose: String,
    value: String,
}

async fn seal(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Json(request): Json<CryptographyRequest>,
) -> Response {
    cryptography(&gateway, &headers, request, true).await
}

async fn open(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Json(request): Json<CryptographyRequest>,
) -> Response {
    cryptography(&gateway, &headers, request, false).await
}

/// 进程只提交明文或密文，历史密钥始终留在宿主 Keyring 内。
async fn cryptography(
    gateway: &Gateway,
    headers: &HeaderMap,
    request: CryptographyRequest,
    sealing: bool,
) -> Response {
    let result = async {
        let components = active(gateway, headers).await?;
        let bundle = components
            .bundle(gateway.start.source, &gateway.start.tenant)
            .await?;
        ensure!(
            bundle.manifest().plugin.capabilities.cryptography,
            "process 未获加密能力"
        );
        let value = STANDARD.decode(request.value).context("加密载荷无效")?;
        let scope = InvocationScope {
            source_id: gateway.start.source.to_string(),
            revision: gateway.start.revision.clone(),
            context: RequestContext {
                tenant_id: Some(gateway.start.tenant.clone()),
                ..Default::default()
            },
            grants: Default::default(),
        };
        let value = if sealing {
            components.keyring.seal(&scope, &request.purpose, &value)?
        } else {
            components.keyring.open(&scope, &request.purpose, &value)?
        };
        Ok::<_, anyhow::Error>(STANDARD.encode(value))
    }
    .await;
    match result {
        Ok(value) => Json(serde_json::json!({ "value": value })).into_response(),
        Err(_) => (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "加密操作未授权或数据无效" })),
        )
            .into_response(),
    }
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

/// 代表进程插件上报用量。宿主未接入计费时返回 204，不视为失败。
async fn meter(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Json(request): Json<MeterRequest>,
) -> Response {
    match meter_inner(&gateway, &headers, request).await {
        Ok(Some(outcome)) => {
            Json(serde_json::json!({"status": 200, "body": outcome})).into_response()
        }
        Ok(None) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "用量上报未授权"})),
        )
            .into_response(),
    }
}

async fn meter_inner(
    gateway: &Gateway,
    headers: &HeaderMap,
    request: MeterRequest,
) -> Result<Option<crate::identity::MeterOutcome>> {
    let components = active(gateway, headers).await?;
    ensure!(
        request.tenant_id == gateway.start.tenant,
        "计量租户与实例不一致"
    );
    ensure!(
        !request.source_id.is_empty()
            && request.resource.len() <= 64
            && request.quantity > 0
            && request.idempotency_key.len() <= 128,
        "用量上报参数无效"
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
    components
        .identity
        .meter(
            &request.tenant_id,
            &request.user_id,
            &request.source_id,
            &request.resource,
            request.quantity,
            &request.idempotency_key,
        )
        .await
}

async fn egress(State(gateway): State<Arc<Gateway>>, headers: HeaderMap, body: Bytes) -> Response {
    match egress_inner(
        gateway,
        headers,
        Some(body),
        GenerationProtocol::ChatCompletions,
    )
    .await
    {
        Ok(response) => response,
        Err(_) => (StatusCode::BAD_GATEWAY, "模型出站不可用或未授权").into_response(),
    }
}

async fn models(State(gateway): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    match egress_inner(gateway, headers, None, GenerationProtocol::Responses).await {
        Ok(response) => response,
        Err(_) => (StatusCode::BAD_GATEWAY, "模型列表不可用或未授权").into_response(),
    }
}

/// 固定协议入口由宿主选择，插件不能传任意上游路径。
enum GenerationProtocol {
    ChatCompletions,
    Responses,
}

async fn responses(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match egress_inner(gateway, headers, Some(body), GenerationProtocol::Responses).await {
        Ok(response) => response,
        Err(_) => (StatusCode::BAD_GATEWAY, "模型服务不可用或未授权").into_response(),
    }
}

async fn egress_inner(
    gateway: Arc<Gateway>,
    headers: HeaderMap,
    body: Option<Bytes>,
    protocol: GenerationProtocol,
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
        match protocol {
            GenerationProtocol::Responses => responses_request(&gateway.client, endpoint, body)?,
            GenerationProtocol::ChatCompletions => {
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
            }
        }
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

/// 校验 Responses 请求后构造固定路径，调用者先完成活动版本和基址授权。
fn responses_request(
    client: &reqwest::Client,
    endpoint: &str,
    body: Bytes,
) -> Result<reqwest::RequestBuilder> {
    let payload: serde_json::Value = serde_json::from_slice(&body)?;
    ensure!(
        payload["stream"] == true
            && payload["model"]
                .as_str()
                .is_some_and(|value| !value.is_empty() && value.len() <= 256)
            && (payload["input"].is_array() || payload["input"].is_string())
            && payload.get("messages").is_none(),
        "Responses 模型请求无效"
    );
    Ok(client
        .post(format!("{endpoint}/responses"))
        .header("content-type", "application/json")
        .body(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn forwards_responses_items_to_fixed_path_and_rejects_chat_body() -> Result<()> {
        let body = json!({"model":"fixture","input":[{"type":"function_call_output","call_id":"call_1","output":"ok"}],"stream":true,"store":false});
        let expected = body.clone();
        let upstream = Router::new().route(
            "/v1/responses",
            post(move |Json(actual): Json<serde_json::Value>| {
                let expected = expected.clone();
                async move {
                    assert_eq!(actual, expected);
                    (
                        [("content-type", "text/event-stream")],
                        "data: {\"type\":\"response.completed\"}\n\n",
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = format!("http://{}/v1", listener.local_addr()?);
        let server = tokio::spawn(async move { axum::serve(listener, upstream).await });
        let client = reqwest::Client::new();
        let response = responses_request(&client, &endpoint, serde_json::to_vec(&body)?.into())?
            .send()
            .await?;
        assert!(response.status().is_success());
        assert!(response.text().await?.contains("response.completed"));
        let invalid = json!({"model":"fixture","messages":[],"stream":true});
        assert!(
            responses_request(&client, &endpoint, serde_json::to_vec(&invalid)?.into()).is_err()
        );
        server.abort();
        Ok(())
    }
}
