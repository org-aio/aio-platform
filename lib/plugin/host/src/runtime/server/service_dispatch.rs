use crate::identity::SessionContext;
use anyhow::Context as _;

use super::{
    RuntimeState, http_error::RuntimeError, process::ProcessResponse, store::ServiceBinding,
};
use crate::runtime::{PluginRequest, PluginRuntime};

pub(super) struct ServiceCall<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub query: Option<&'a str>,
    pub body: Vec<u8>,
    pub content_type: Option<&'a str>,
}

pub(super) async fn dispatch(
    state: &RuntimeState,
    session: &SessionContext,
    source_id: &str,
    binding: &ServiceBinding,
    call: ServiceCall<'_>,
) -> Result<ProcessResponse, RuntimeError> {
    ensure_route_allowed(&binding.routes, call.path.trim_start_matches('/'))?;
    match binding.runtime {
        PluginRuntime::WasmComponent => {
            let request = serde_json::to_string(&PluginRequest::ServiceRequest {
                method: call.method.to_owned(),
                path: call.path.to_owned(),
                query: call.query.map(str::to_owned),
                body: String::from_utf8(call.body).context("Wasm Component 请求体必须是 UTF-8")?,
                tenant_id: session.tenant_id.clone(),
                user_id: session.user_id.clone(),
            })
            .context("序列化 Wasm 服务请求失败")?;
            let manager = state.wasm.clone();
            let tenant_id = session.tenant_id.clone();
            let source_id = source_id.to_owned();
            let revision = binding.revision.clone();
            let output = tokio::task::spawn_blocking(move || {
                manager.handle(&tenant_id, &source_id, &revision, request)
            })
            .await
            .context("等待 Wasm 请求处理失败")??;
            Ok(ProcessResponse {
                status: reqwest::StatusCode::from_u16(output.status)
                    .context("Wasm Component 返回了无效状态码")?,
                content_type: Some(output.content_type),
                body: output.body.into_bytes(),
            })
        }
        PluginRuntime::Process => {
            let endpoint = binding
                .endpoint
                .as_deref()
                .context("process 插件缺少活动 endpoint")?;
            Ok(state
                .process
                .request(
                    endpoint,
                    call.method,
                    call.path,
                    call.query,
                    call.body,
                    call.content_type,
                    &session.tenant_id,
                    &session.user_id,
                )
                .await?)
        }
        _ => Err(RuntimeError::not_found("当前插件不提供动态服务")),
    }
}

pub(super) fn ensure_route_allowed(routes: &[String], path: &str) -> Result<(), RuntimeError> {
    if routes
        .iter()
        .any(|route| path == route || path.starts_with(&format!("{route}/")))
    {
        return Ok(());
    }
    Err(RuntimeError::not_found("插件清单未声明该服务路由"))
}
