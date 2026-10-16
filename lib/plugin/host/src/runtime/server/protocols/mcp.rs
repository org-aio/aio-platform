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
use sha2::{Digest, Sha256};

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
        let (_, contracts) = access::catalog(&self.0, session(&context)?)
            .await
            .map_err(|_| ErrorData::internal_error("读取增强目录失败", None))?;
        let tools = contracts
            .into_iter()
            .map(|contract| {
                // 路径不允许点号，转换是可逆的；它是协议符号，不承担原生插件身份。
                let name = contract.path.trim_start_matches('/').replace('/', ".");
                let input = envelope("input", contract.input_schema)?;
                let output = envelope("output", contract.output_schema)?;
                let mut tool = Tool::new(name, format!("调用运行时增强 {}", contract.path), input);
                tool.output_schema = Some(Arc::new(output));
                Ok::<_, ErrorData>(tool)
            })
            .collect::<Result<Vec<_>, _>>()?;
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
            None,
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

// 增强接收任意 JSON，MCP 则要求对象参数；嵌入资源必须有独立 base，
// 否则业务 schema 的 #/$defs 引用会错误地解析到 MCP 外层对象。
fn envelope(
    field: &str,
    schema: schemars::Schema,
) -> Result<serde_json::Map<String, Value>, ErrorData> {
    let mut value = serde_json::to_value(schema)
        .map_err(|_| ErrorData::internal_error("编码增强 schema 失败", None))?;
    let bytes = serde_json::to_vec(&value)
        .map_err(|_| ErrorData::internal_error("编码增强 schema 失败", None))?;
    let digest = format!("{:x}", Sha256::digest(bytes));
    let base = reqwest::Url::parse(&format!(
        "https://schemas.aio.invalid/enhancements/{digest}/{field}"
    ))
    .map_err(|_| ErrorData::internal_error("增强 schema 地址无效", None))?;
    if let Value::Object(object) = &mut value {
        let same_resource = object
            .get("$id")
            .and_then(Value::as_str)
            .map(|id| {
                base.join(id).map(|mut resolved| {
                    resolved.set_fragment(None);
                    resolved == base
                })
            })
            .transpose()
            .map_err(|_| ErrorData::internal_error("增强 schema 标识无效", None))?
            .unwrap_or(true);
        // 空 id、空片段以及解析后等于外层的相对 id 也必须建立独立资源。
        if same_resource {
            object.insert("$id".into(), json!(format!("{field}.schema.json")));
        }
    }
    let mut properties = serde_json::Map::new();
    properties.insert(field.into(), value);
    let mut required = vec![field];
    if field == "output" {
        properties.insert("revision".into(), json!({"type":"string"}));
        required.push("revision");
    }
    serde_json::from_value(json!({
        "$schema":"https://json-schema.org/draft/2020-12/schema",
        "$id":base.as_str(),
        "type":"object", "properties":properties, "required":required, "additionalProperties":false
    }))
    .map_err(|_| ErrorData::internal_error("增强参数契约无效", None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_schema_ids_keep_local_references() -> anyhow::Result<()> {
        for id in [
            None,
            Some(""),
            Some("#"),
            Some("input"),
            Some("relative.json"),
            Some("https://example.test/input"),
        ] {
            let mut schema = json!({"$defs":{"value":{"type":"integer"}},"$ref":"#/$defs/value"});
            if let Some(id) = id {
                schema["$id"] = json!(id);
            }
            let original = jsonschema::options().offline().build(&schema)?;
            assert!(original.is_valid(&json!(3)));
            let wrapped = envelope("input", schema.try_into()?)?;
            let validator = jsonschema::options().offline().build(&json!(wrapped))?;
            assert!(validator.is_valid(&json!({"input":3})), "schema id: {id:?}");
            assert!(
                !validator.is_valid(&json!({"input":"wrong"})),
                "schema id: {id:?}"
            );
        }
        Ok(())
    }
}
