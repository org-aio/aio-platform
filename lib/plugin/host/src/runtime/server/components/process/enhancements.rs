use anyhow::{Context, Result};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use az_plugin_contract::{
    EnhancementRequest, EnhancementResponse, InvocationScope, RequestContext,
};
use az_plugin_runtime::HostServices;
use std::sync::Arc;

use super::model::Gateway;

/// Sidecar 复用活动版本票据和当前安装包能力，不能在请求中伪造调用归属。
pub(super) async fn invoke(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Json(request): Json<EnhancementRequest>,
) -> Response {
    let result: Result<EnhancementResponse> = async {
        let _permit = gateway.quota.try_acquire()?;
        let components = super::broker::active(&gateway, &headers).await?;
        let source = gateway.start.source.to_string();
        let context_id = headers
            .get("x-aio-context")
            .map(|header| header.to_str())
            .transpose()
            .context("交互上下文无效")?;
        let context = match context_id {
            Some(id) => components
                .services
                .interactive(id, &gateway.start.tenant, &source)?,
            None => RequestContext {
                tenant_id: Some(gateway.start.tenant.clone()),
                ..Default::default()
            },
        };
        let scope = InvocationScope {
            source_id: source,
            revision: gateway.start.revision.clone(),
            context,
            // 固定为创建此进程时的能力，避免并发升级让旧票据取得新版额外权限。
            grants: az_plugin_contract::CapabilityGrants {
                enhancements: gateway.enhancements.clone(),
                runtime: gateway.runtime_capabilities.clone(),
                ..Default::default()
            },
        };
        if context_id.is_some() {
            components.services.enhance(&scope, request).await
        } else {
            components
                .enhancements
                .as_ref()
                .context("宿主未安装增强服务")?
                .invoke(&scope, request, None)
                .await
        }
    }
    .await;
    match result {
        Ok(response) => Json(response).into_response(),
        Err(_) => (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error":"增强能力不可用或调用未授权"})),
        )
            .into_response(),
    }
}

#[cfg(all(test, feature = "postgres"))]
#[path = "enhancements_tests.rs"]
mod tests;
