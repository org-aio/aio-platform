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
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    },
    service::RequestContext,
    transport::{
        StreamableHttpService,
        streamable_http_server::{StreamableHttpServerConfig, session::local::LocalSessionManager},
    },
};
use serde_json::{Value, json};

use super::{super::RuntimeState, access};
use crate::identity::SessionContext;

struct McpEnhancements(RuntimeState);

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
        move || Ok(McpEnhancements(runtime.clone())),
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

impl ServerHandler for McpEnhancements {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("工具对应当前租户获准使用的版本化运行时增强；调用传入 input，结果包含实际执行的 revision。升级后重新获取工具目录。")
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        if request.is_some_and(|request| request.cursor.is_some()) {
            return Err(ErrorData::invalid_params("增强目录不使用分页游标", None));
        }
        let (_, paths) = access::catalog(&self.0, session(&context)?)
            .await
            .map_err(|_| ErrorData::internal_error("读取增强目录失败", None))?;
        let schema = serde_json::from_value::<serde_json::Map<String, Value>>(json!({
            "type": "object", "properties": {"input": {}}, "required": ["input"], "additionalProperties": false
        })).map_err(|_| ErrorData::internal_error("增强参数契约无效", None))?;
        let tools = paths
            .into_iter()
            .map(|path| {
                // 路径不允许点号，转换是可逆的；它是协议符号，不承担原生插件身份。
                let name = path.trim_start_matches('/').replace('/', ".");
                Tool::new(name, format!("调用运行时增强 {path}"), schema.clone())
            })
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let path = format!("/{}", request.name.replace('.', "/"));
        az_plugin_contract::validate_enhancement_path(&path)
            .map_err(|_| ErrorData::invalid_params("增强工具名称无效", None))?;
        if request.name.contains('/') {
            return Err(ErrorData::invalid_params("增强工具名称无效", None));
        }
        let mut arguments = request.arguments.unwrap_or_default();
        if arguments.len() != 1 || !arguments.contains_key("input") {
            return Err(ErrorData::invalid_params("必须且只能提供 input 参数", None));
        }
        let input = arguments
            .remove("input")
            .ok_or_else(|| ErrorData::invalid_params("缺少 input 参数", None))?;
        match access::invoke(
            &self.0,
            session(&context)?,
            EnhancementRequest { path, input },
        )
        .await
        {
            Ok(response) => Ok(CallToolResult::structured(
                json!({"revision":response.revision,"output":response.output}),
            )
            .into()),
            Err(_) => Ok(CallToolResult::structured_error(
                json!({"error":"增强未授权、未激活或执行失败"}),
            )
            .into()),
        }
    }
}
