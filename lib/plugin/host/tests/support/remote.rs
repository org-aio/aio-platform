// 协议入口与原生异步入口共用真实上游；各用例只使用所需的观察量。

use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use anyhow::Result;
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::post,
};
use prost::Message;
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
    },
    service::RequestContext,
    transport::{
        StreamableHttpService,
        streamable_http_server::{StreamableHttpServerConfig, session::local::LocalSessionManager},
    },
};
use serde_json::{Value, json};

#[derive(Default)]
pub struct Counts {
    pub calls: AtomicUsize,
    pub active: AtomicUsize,
    pub changed: AtomicBool,
    pub credentials: AtomicUsize,
    pub deadlines: AtomicUsize,
    pub last_timeout_micros: AtomicUsize,
    pub release: tokio::sync::Notify,
    pub pause_catalog: AtomicBool,
    pub catalog_entered: tokio::sync::Notify,
    pub catalog_release: tokio::sync::Notify,
    pub catalog_active: AtomicUsize,
}

struct Discovery(Arc<Counts>);
impl Drop for Discovery {
    fn drop(&mut self) {
        self.0.catalog_active.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Active(Arc<Counts>);
impl Active {
    fn enter(counts: Arc<Counts>) -> Self {
        counts.calls.fetch_add(1, Ordering::SeqCst);
        counts.active.fetch_add(1, Ordering::SeqCst);
        Self(counts)
    }
}
impl Drop for Active {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct Upstream {
    pub base: String,
    pub counts: Arc<Counts>,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
    root: tempfile::TempDir,
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.server.abort();
    }
}

pub fn schema() -> Value {
    json!({"type":"object","properties":{"value":{"type":"integer"}},"required":["value"],"additionalProperties":false})
}

impl Upstream {
    pub async fn open() -> Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let counts = Arc::new(Counts::default());
        let cloned = counts.clone();
        let mcp = StreamableHttpService::new(
            move || Ok(Tools(cloned.clone())),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default()
                .with_legacy_session_mode(false)
                .with_json_response(true),
        );
        let cloned = counts.clone();
        let stateful = StreamableHttpService::new(
            move || Ok(Tools(cloned.clone())),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default()
                .with_legacy_session_mode(true)
                .with_json_response(true),
        );
        let router = Router::new()
            .route_service("/mcp", mcp)
            .route_service("/stateful", stateful)
            .route("/external.calculator.Math/Double", post(grpc))
            .route(
                "/oversized",
                post(|| async {
                    let chunks = futures_util::stream::iter(
                        (0..32).map(|_| Ok::<_, std::io::Error>(vec![b'x'; 64 * 1024])),
                    );
                    (
                        [("content-type", "text/event-stream")],
                        Body::from_stream(chunks),
                    )
                }),
            )
            .route(
                "/redirect",
                post(|| async { (StatusCode::TEMPORARY_REDIRECT, [("location", "/mcp")]) }),
            )
            .layer(axum::middleware::from_fn_with_state(
                counts.clone(),
                authenticate,
            ))
            .with_state(counts.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await });
        let root = tempfile::tempdir()?;
        std::fs::write(root.path().join("calculator.bin"), descriptor())?;
        Ok(Self {
            base,
            counts,
            server,
            root,
        })
    }

    pub fn imports(&self) -> Vec<Value> {
        ["mcp", "grpc"].into_iter().map(|protocol| {
            let transport = if protocol == "mcp" {
                json!({"protocol":"mcp","tool":"double","output":"structured"})
            } else {
                json!({"protocol":"grpc","descriptor_set":self.root.path().join("calculator.bin"),"service":"external.calculator.Math","method":"Double"})
            };
            let endpoint = if protocol == "mcp" { format!("{}/mcp", self.base) } else { self.base.clone() };
            json!({"tenant_id":"test","capability":{"path":format!("/v1/remote/{protocol}"),"description":"外部计算器","effect":"read","input_schema":schema(),"output_schema":schema()},"endpoint":endpoint,"credential_env":"AIO_REMOTE_TEST_CREDENTIAL","transport":transport})
        }).collect()
    }
}

async fn authenticate(
    State(counts): State<Arc<Counts>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if request.headers().contains_key("cookie") || request.headers().contains_key("x-aio-context") {
        return StatusCode::FORBIDDEN.into_response();
    }
    if request
        .headers()
        .get("authorization")
        .is_none_or(|v| v != "Bearer remote-test")
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    counts.credentials.fetch_add(1, Ordering::SeqCst);
    if let Some(timeout) = request.headers().get("grpc-timeout") {
        counts.deadlines.fetch_add(1, Ordering::SeqCst);
        let timeout = timeout
            .to_str()
            .ok()
            .and_then(timeout_micros)
            .unwrap_or_default();
        counts.last_timeout_micros.store(timeout, Ordering::SeqCst);
    }
    next.run(request).await
}

struct Tools(Arc<Counts>);
impl ServerHandler for Tools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        if self.0.pause_catalog.load(Ordering::SeqCst) {
            self.0.catalog_active.fetch_add(1, Ordering::SeqCst);
            let _discovery = Discovery(self.0.clone());
            self.0.catalog_entered.notify_one();
            tokio::select! {
                _ = self.0.catalog_release.notified() => {},
                _ = context.ct.cancelled() => return Err(ErrorData::internal_error("发现已取消", None)),
            }
        }
        let mut input = schema();
        if self.0.changed.load(Ordering::SeqCst) {
            input["properties"]["value"]["type"] = json!("string");
        }
        let mut tool = Tool::new(
            "double",
            "测试外部工具",
            input.as_object().cloned().unwrap_or_default(),
        )
        .with_annotations(ToolAnnotations::new().read_only(true));
        tool.output_schema = Some(Arc::new(schema().as_object().cloned().unwrap_or_default()));
        Ok(ListToolsResult::with_all_items(vec![tool]))
    }

    async fn call_tool(
        &self,
        params: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if params.name != "double" {
            return Err(ErrorData::invalid_params("未知工具", None));
        }
        let value = params
            .arguments
            .as_ref()
            .and_then(|args| args.get("value"))
            .and_then(Value::as_i64)
            .ok_or_else(|| ErrorData::invalid_params("缺少 value", None))?;
        let _active = Active::enter(self.0.clone());
        if value == -5 {
            tokio::select! {
                _ = self.0.release.notified() => {},
                _ = context.ct.cancelled() => return Err(ErrorData::internal_error("已取消", None)),
            }
        }
        if value == -1 {
            context.ct.cancelled().await;
            return Err(ErrorData::internal_error("已取消", None));
        }
        if value == -2 {
            return Ok(CallToolResult::error(vec![rmcp::model::ContentBlock::text(
                "上游业务失败",
            )])
            .into());
        }
        let output = if value == -3 {
            json!({"value":"x".repeat(2*1024*1024)})
        } else if value == -4 {
            json!({"value":"bad"})
        } else {
            json!({"value":value*2})
        };
        Ok(CallToolResult::structured(output).into())
    }
}

#[derive(Clone, PartialEq, Message)]
struct Number {
    #[prost(int32, tag = "1")]
    value: i32,
}

struct Double(Arc<Counts>);
impl tonic::server::UnaryService<Number> for Double {
    type Response = Number;
    type Future =
        Pin<Box<dyn Future<Output = Result<tonic::Response<Number>, tonic::Status>> + Send>>;
    fn call(&mut self, request: tonic::Request<Number>) -> Self::Future {
        let counts = self.0.clone();
        Box::pin(async move {
            let _active = Active::enter(counts.clone());
            let value = request.into_inner().value;
            if value == -5 {
                counts.release.notified().await;
            }
            if value == -1 {
                std::future::pending::<()>().await;
            }
            if value == -2 {
                return Err(tonic::Status::unavailable("上游业务失败"));
            }
            Ok(tonic::Response::new(Number { value: value * 2 }))
        })
    }
}

async fn grpc(State(counts): State<Arc<Counts>>, request: Request<Body>) -> Response {
    let codec = tonic_prost::ProstCodec::<Number, Number>::default();
    tonic::server::Grpc::new(codec)
        .unary(Double(counts), request)
        .await
        .map(Body::new)
}

fn descriptor() -> Vec<u8> {
    use prost_types::{
        DescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet,
        MethodDescriptorProto, ServiceDescriptorProto,
    };
    FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("calculator.proto".into()),
            package: Some("external.calculator".into()),
            syntax: Some("proto3".into()),
            message_type: vec![DescriptorProto {
                name: Some("Number".into()),
                field: vec![FieldDescriptorProto {
                    name: Some("value".into()),
                    number: Some(1),
                    label: Some(1),
                    r#type: Some(5),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            service: vec![ServiceDescriptorProto {
                name: Some("Math".into()),
                method: vec![MethodDescriptorProto {
                    name: Some("Double".into()),
                    input_type: Some(".external.calculator.Number".into()),
                    output_type: Some(".external.calculator.Number".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
    .encode_to_vec()
}

// 独立观察线上 grpc-timeout，避免复用被测入口的解析函数。
fn timeout_micros(value: &str) -> Option<usize> {
    if !value.is_ascii() || value.len() < 2 {
        return None;
    }
    let (amount, unit) = value.split_at(value.len() - 1);
    let amount: u64 = amount.parse().ok()?;
    let nanos = match unit {
        "H" => 3_600_000_000_000,
        "M" => 60_000_000_000,
        "S" => 1_000_000_000,
        "m" => 1_000_000,
        "u" => 1_000,
        "n" => 1,
        _ => return None,
    };
    usize::try_from(amount.checked_mul(nanos)? / 1000).ok()
}
