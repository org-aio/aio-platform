use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Request, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use az_plugin_contract::EnhancementRequest;
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig,
    },
    service::RequestContext,
    transport::{
        StreamableHttpService,
        streamable_http_server::{StreamableHttpServerConfig, session::local::LocalSessionManager},
    },
};
use serde_json::json;

use super::{
    super::RuntimeState,
    access,
    mcp_catalog::{self, Target},
};
use crate::identity::SessionContext;

struct McpCapabilities(RuntimeState);

pub(in crate::runtime::server) fn router(state: RuntimeState) -> Router<RuntimeState> {
    let mut config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_max_request_body_bytes(az_plugin_enhancement::MAX_VALUE_BYTES * 2)
        .enforce_origin_validation();
    // 公开地址已在 RuntimeState 初始化时校验；仅接受该宿主和同源浏览器。
    if let Ok(origin) = reqwest::Url::parse(&state.config.public_origin)
        && let Some(host) = origin.host_str()
    {
        config = config.with_allowed_hosts([host]);
        if let Some(port) = origin.port_or_known_default() {
            config = config.with_allowed_origins([format!("{}://{host}:{port}", origin.scheme())]);
        }
    }
    let runtime = state.clone();
    let service = StreamableHttpService::new(
        move || Ok(McpCapabilities(runtime.clone())),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    Router::new()
        .route_service("/api/runtime/mcp", service)
        .route_layer(axum::middleware::from_fn_with_state(state, authenticate))
}

async fn authenticate(
    State(state): State<RuntimeState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    match access::session(&state, request.headers()).await {
        Ok(session) => {
            request.extensions_mut().insert(session);
            next.run(request).await
        }
        Err(error) => error.into_response(),
    }
}

fn session(context: &RequestContext<RoleServer>) -> Result<&SessionContext, ErrorData> {
    context
        .extensions
        .get::<Parts>()
        .and_then(|parts| parts.extensions.get::<SessionContext>())
        .ok_or_else(|| ErrorData::internal_error("请求缺少已验证身份", None))
}

impl ServerHandler for McpCapabilities {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("工具对应当前租户获准使用的能力。runtime.* 调用原生或导入能力，其余名称调用脚本增强。统一传入 input；增强结果另含实际 revision。目录可分页，变更后重新发现。")
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        mcp_catalog::list(&self.0, session(&context)?, request).await
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let target = mcp_catalog::target(&request.name)?;
        let mut arguments = request.arguments.unwrap_or_default();
        if arguments.len() != 1 || !arguments.contains_key("input") {
            return Err(ErrorData::invalid_params("必须且只能提供 input 参数", None));
        }
        let input = arguments
            .remove("input")
            .ok_or_else(|| ErrorData::invalid_params("缺少 input 参数", None))?;
        let session = session(&context)?;
        let operation = async {
            match target {
                Target::Capability(path) => access::invoke_capability(
                    &self.0,
                    session,
                    EnhancementRequest { path, input },
                    None,
                )
                .await
                .map(|output| json!({"output":output})),
                Target::Enhancement(path) => access::invoke(
                    &self.0,
                    session,
                    EnhancementRequest { path, input },
                    None,
                )
                .await
                .map(|response| json!({"revision":response.revision,"output":response.output})),
            }
        };
        let result = tokio::select! {
            _ = context.ct.cancelled() => return Err(ErrorData::internal_error("能力调用已取消", None)),
            result = operation => result,
        };
        match result {
            Ok(output) => Ok(CallToolResult::structured(output).into()),
            Err(_) => Ok(CallToolResult::structured_error(
                json!({"error":"能力未授权、不可用或执行失败"}),
            )
            .into()),
        }
    }
}
