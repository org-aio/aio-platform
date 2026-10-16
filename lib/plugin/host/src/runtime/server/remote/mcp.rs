use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use rmcp::{
    RoleClient, ServiceExt,
    model::{
        CallToolRequest, CallToolRequestParams, ClientRequest, ListToolsRequest,
        PaginatedRequestParams, ServerResult, Tool,
    },
    service::{PeerRequestOptions, RunningService},
    transport::{
        StreamableHttpClientTransport, common::client_side_sse::NeverRetry,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::Value;

use super::super::plugins::{CapabilityEffect, PluginTasks, RuntimeCapability};
use super::{
    configuration::{MAX_WIRE_BYTES, McpOutput},
    mcp_http::BoundedClient,
};

#[derive(Clone)]
pub(super) struct Mcp {
    client: BoundedClient,
    endpoint: String,
    tool: String,
    output: McpOutput,
    capability: RuntimeCapability,
    // 清理也持有配额，避免频繁取消在后台积累尚未结束的会话。
    calls: Arc<tokio::sync::Semaphore>,
}

impl Mcp {
    pub async fn open(
        endpoint: &str,
        auth: Option<axum::http::HeaderValue>,
        tool: String,
        output: McpOutput,
        capability: &RuntimeCapability,
        tasks: &PluginTasks,
    ) -> Result<Self> {
        ensure!(!tool.is_empty() && tool.len() <= 256, "MCP 工具名称无效");
        let binding = Self {
            client: BoundedClient::new(auth)?,
            endpoint: endpoint.into(),
            tool,
            output,
            capability: capability.clone(),
            calls: Arc::new(tokio::sync::Semaphore::new(4)),
        };
        binding
            .session(None, tasks, Instant::now() + Duration::from_secs(30), None)
            .await?;
        Ok(binding)
    }

    async fn connect(&self) -> Result<RunningService<RoleClient, ()>> {
        let mut config = StreamableHttpClientTransportConfig::with_uri(self.endpoint.clone());
        config.retry_config = Arc::new(NeverRetry::default());
        config.reinit_on_expired_session = false;
        config.max_concurrent_requests = 1;
        config.max_sse_event_size = MAX_WIRE_BYTES;
        let transport = StreamableHttpClientTransport::with_client(self.client.clone(), config);
        ().serve(transport).await.context("连接远端 MCP 失败")
    }

    async fn validate_tool(
        &self,
        connection: &RunningService<RoleClient, ()>,
        sender: &mut tokio::sync::oneshot::Sender<Result<SessionOutput>>,
        cancellation: &tokio_util::sync::CancellationToken,
        deadline: Instant,
    ) -> Result<bool> {
        let mut cursor = None;
        let mut visited = HashSet::new();
        let mut selected = None;
        let mut bytes = 0;
        for _ in 0..16 {
            let request = ListToolsRequest {
                params: cursor.map(|cursor| {
                    let mut params = PaginatedRequestParams::default();
                    params.cursor = Some(cursor);
                    params
                }),
                ..Default::default()
            };
            let Some(response) = cancellable_request(
                connection,
                sender,
                ClientRequest::ListToolsRequest(request),
                cancellation,
                deadline,
            )
            .await?
            else {
                return Ok(false);
            };
            let ServerResult::ListToolsResult(page) = response else {
                anyhow::bail!("MCP 发现返回了错误的消息类型");
            };
            bytes += serde_json::to_vec(&page)?.len();
            ensure!(bytes <= MAX_WIRE_BYTES, "MCP 工具目录超过配额");
            for tool in page.tools {
                if tool.name == self.tool {
                    ensure!(selected.is_none(), "MCP 目录包含重复工具名称");
                    selected = Some(tool);
                }
            }
            cursor = page.next_cursor;
            let Some(next) = &cursor else {
                self.check_contract(&selected.context("远端 MCP 工具不存在")?)?;
                return Ok(true);
            };
            ensure!(visited.insert(next.clone()), "MCP 工具目录分页循环");
        }
        anyhow::bail!("MCP 工具目录页数超过配额")
    }

    fn check_contract(&self, tool: &Tool) -> Result<()> {
        ensure!(
            Value::Object(tool.input_schema.as_ref().clone())
                == *self.capability.input_schema.as_value(),
            "MCP 输入 schema 与导入契约不一致"
        );
        if matches!(self.output, McpOutput::Structured) {
            let schema = tool
                .output_schema
                .as_ref()
                .context("MCP 工具未声明结构化输出 schema")?;
            ensure!(
                Value::Object(schema.as_ref().clone()) == *self.capability.output_schema.as_value(),
                "MCP 输出 schema 与导入契约不一致"
            );
        }
        let annotations = tool.annotations.as_ref();
        let read = annotations.and_then(|a| a.read_only_hint) == Some(true);
        match self.capability.effect {
            CapabilityEffect::Read => ensure!(read, "MCP 工具未确认只读约定"),
            CapabilityEffect::IdempotentWrite => ensure!(
                read || annotations.and_then(|a| a.idempotent_hint) == Some(true),
                "MCP 工具未确认幂等约定"
            ),
            CapabilityEffect::Write => {}
        }
        Ok(())
    }

    pub async fn invoke(
        &self,
        input: &Value,
        tasks: &PluginTasks,
        deadline: Instant,
    ) -> Result<Value> {
        let permit = self
            .calls
            .clone()
            .try_acquire_owned()
            .context("MCP 调用及清理并发已满")?;
        let arguments = input
            .as_object()
            .context("MCP 工具参数必须为 JSON 对象")?
            .clone();
        let params = CallToolRequestParams::new(self.tool.clone()).with_arguments(arguments);
        let SessionOutput::Tool(response) = self
            .session(Some(params), tasks, deadline, Some(permit))
            .await?
        else {
            anyhow::bail!("MCP 会话没有返回工具结果");
        };
        let ServerResult::CallToolResult(result) = *response else {
            anyhow::bail!("MCP 工具返回了未声明的交互或后台任务");
        };
        ensure!(result.is_error != Some(true), "远端 MCP 工具执行失败");
        match self.output {
            McpOutput::Structured => result.structured_content.context("MCP 工具缺少结构化结果"),
            McpOutput::Envelope => Ok(serde_json::to_value(result)?),
        }
    }

    async fn session(
        &self,
        params: Option<CallToolRequestParams>,
        tasks: &PluginTasks,
        deadline: Instant,
        permit: Option<tokio::sync::OwnedSemaphorePermit>,
    ) -> Result<SessionOutput> {
        let connection = self.connect().await?;
        let binding = self.clone();
        let cancellation = tasks.cancellation();
        let (mut sender, receiver) = tokio::sync::oneshot::channel();
        tasks.spawn(async move {
            let _permit = permit;
            let operation = async {
                if !binding
                    .validate_tool(&connection, &mut sender, &cancellation, deadline)
                    .await?
                {
                    return Ok(None);
                }
                let Some(params) = params else {
                    return Ok(Some(SessionOutput::Validated));
                };
                let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
                let result =
                    cancellable_request(&connection, &mut sender, request, &cancellation, deadline)
                        .await?;
                Ok(result.map(|result| SessionOutput::Tool(Box::new(result))))
            }
            .await;
            let result = finish(connection, operation).await;
            if let Ok(None) = result {
                return Ok(());
            }
            let result = result.and_then(|value| value.context("MCP 会话已取消"));
            if let Err(result) = sender.send(result) {
                // 调用方已退出时，清理错误由插件任务作用域保留并在关闭时报告。
                result?;
            }
            Ok(())
        })?;
        receiver.await.context("MCP 会话已取消或超过截止时间")?
    }
}

enum SessionOutput {
    Validated,
    // 工具消息较大，使用 Box 避免纯发现结果也占用同样的栈空间。
    Tool(Box<ServerResult>),
}

// 有状态服务器的发现和工具请求均可能在 HTTP 连接结束后继续执行；先取消，再关闭会话。
async fn cancellable_request(
    connection: &RunningService<RoleClient, ()>,
    sender: &mut tokio::sync::oneshot::Sender<Result<SessionOutput>>,
    message: ClientRequest,
    cancellation: &tokio_util::sync::CancellationToken,
    deadline: Instant,
) -> Result<Option<ServerResult>> {
    if sender.is_closed() || cancellation.is_cancelled() || Instant::now() >= deadline {
        return Ok(None);
    }
    // 一条请求只发送一次，不驱动交互轮次，也不进行任务重提交。
    let mut request = connection
        .peer()
        .send_cancellable_request(message, PeerRequestOptions::no_options())
        .await?;
    tokio::select! {
        response = &mut request.rx => return Ok(Some(response.context("MCP 连接已关闭")??)),
        _ = sender.closed() => {},
        _ = cancellation.cancelled() => {},
        _ = tokio::time::sleep_until(deadline.into()) => {},
    }
    tokio::time::timeout(
        Duration::from_secs(2),
        request.cancel(Some("宿主调用取消或超过截止时间".into())),
    )
    .await
    .context("发送 MCP 取消通知超时")??;
    Ok(None)
}

// 关闭失败不能遮盖真正的调用失败；调用已成功时也明确报告清理失败，禁止重试副作用。
async fn finish<T>(mut connection: RunningService<RoleClient, ()>, result: Result<T>) -> Result<T> {
    let closed = tokio::time::timeout(Duration::from_secs(2), connection.close())
        .await
        .context("关闭远端 MCP 会话超时")
        .and_then(|result| result.map_err(anyhow::Error::from));
    match (result, closed) {
        (Ok(value), Ok(_)) => Ok(value),
        (Err(error), Ok(_)) => Err(error),
        (Ok(_), Err(error)) => Err(error).context("关闭远端 MCP 会话失败"),
        (Err(error), Err(close)) => Err(error.context(format!("关闭远端 MCP 会话也失败: {close}"))),
    }
}
