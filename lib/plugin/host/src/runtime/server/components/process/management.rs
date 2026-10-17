use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use az_plugin_contract::InvocationScope;
use az_plugin_runtime::HostServices;

use super::super::model;
use super::model::Gateway;

/// 进程只提交宿主登记的交互上下文；能力固定为启动该版本时的快照。
pub(super) async fn invoke(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Json(request): Json<model::Request>,
) -> Response {
    match invoke_inner(&gateway, &headers, request).await {
        Ok(response) => Json(model::Response::from(response)).into_response(),
        Err(error) => (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": format!("{error:#}")})),
        )
            .into_response(),
    }
}

async fn invoke_inner(
    gateway: &Gateway,
    headers: &HeaderMap,
    request: model::Request,
) -> Result<az_plugin_contract::transport::Response> {
    let _permit = gateway
        .quota
        .try_acquire()
        .context("process 调用并发已满")?;
    let components = super::broker::active(gateway, headers).await?;
    ensure!(gateway.management, "process 未获管理能力");
    let id = headers
        .get("x-aio-context")
        .context("管理调用缺少交互上下文")?
        .to_str()?;
    let source = gateway.start.source.to_string();
    let context = components
        .services
        .interactive(id, &gateway.start.tenant, &source)?;
    let scope = InvocationScope {
        source_id: source,
        revision: gateway.start.revision.clone(),
        context,
        grants: az_plugin_contract::CapabilityGrants {
            management: gateway.management,
            ..Default::default()
        },
    };
    components
        .services
        .manage(&scope, request.try_into()?)
        .await
}
