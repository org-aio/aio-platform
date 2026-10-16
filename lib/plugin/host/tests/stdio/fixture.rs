//! 真实 MCP stdio 验收进程；协议由官方 SDK 提供，故障模式只用于测试。

use std::{path::PathBuf, process::Stdio, sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
    },
    service::RequestContext,
};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "descendant") {
        std::future::pending::<()>().await;
    }
    let root = PathBuf::from(args.get(1).context("缺少测试目录")?);
    let factor = args.get(2).context("缺少测试倍数")?.parse()?;
    let mut descendant = tokio::process::Command::new(std::env::current_exe()?)
        .arg("descendant")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let pid = std::process::id();
    let record = root.join(format!("{pid}.json"));
    let env_ok = std::env::var("CHILD_TOKEN").ok().as_deref() == Some("stdio-test")
        && std::env::var_os("AIO_MCP_STDIO_HOST_SECRET").is_none()
        && std::env::var_os("HOME").is_none()
        && std::env::var_os("PATH").is_none();
    let data = json!({"parent":pid,"descendant":descendant.id(),"env_ok":env_ok,"cwd_ok":std::env::current_dir()? == root,"literal":args.get(3)});
    let pending = record.with_extension("pending");
    std::fs::write(&pending, serde_json::to_vec(&data)?)?;
    std::fs::rename(pending, record)?;
    ensure!(env_ok, "子进程环境隔离失败");
    let mode = std::fs::read_to_string(root.join("mode"))?;
    match mode.as_str() {
        "handshake-stall" => std::future::pending::<()>().await,
        "crash" => std::process::exit(23),
        "malformed" => {
            tokio::io::stdout()
                .write_all(b"invalid-json secret-canary\n")
                .await?;
            std::future::pending::<()>().await;
        }
        "oversized" => {
            tokio::io::stdout()
                .write_all(&vec![b'x'; 2 * 1024 * 1024])
                .await?;
            std::future::pending::<()>().await;
        }
        _ => {}
    }
    let tools = Tools {
        root,
        pid,
        factor,
        mode,
    };
    let server = tools
        .serve((tokio::io::stdin(), tokio::io::stdout()))
        .await?;
    server.waiting().await?;
    descendant.kill().await?;
    Ok(())
}

struct Tools {
    root: PathBuf,
    pid: u32,
    factor: i64,
    mode: String,
}

fn schema() -> Value {
    json!({"type":"object","properties":{"value":{"type":"integer"}},"required":["value"],"additionalProperties":false})
}

impl ServerHandler for Tools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        std::fs::write(self.root.join(format!("{}.discovered", self.pid)), "")
            .map_err(|_| ErrorData::internal_error("写测试事件失败", None))?;
        if self.mode == "discovery-stall" {
            std::future::pending::<()>().await;
        }
        if self.mode == "flood" {
            let notification = serde_json::to_vec(&json!({"jsonrpc":"2.0","method":"notifications/message","params":{"level":"info","data":"x".repeat(32768)}}))
                .map_err(|_| ErrorData::internal_error("创建测试通知失败", None))?;
            for _ in 0..160 {
                let mut stdout = tokio::io::stdout();
                stdout
                    .write_all(&notification)
                    .await
                    .map_err(|_| ErrorData::internal_error("写通知失败", None))?;
                stdout
                    .write_all(b"\n")
                    .await
                    .map_err(|_| ErrorData::internal_error("写通知失败", None))?;
            }
        }
        let mut input = schema();
        if self.mode == "schema-drift" {
            input["properties"]["value"]["type"] = json!("string");
        }
        let mut tool = Tool::new(
            "double",
            "测试本机工具",
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
        let value = params
            .arguments
            .as_ref()
            .and_then(|args| args.get("value"))
            .and_then(Value::as_i64)
            .ok_or_else(|| ErrorData::invalid_params("缺少 value", None))?;
        std::fs::write(self.root.join(format!("{}.called", self.pid)), "")
            .map_err(|_| ErrorData::internal_error("写测试事件失败", None))?;
        match value {
            -1 => {
                context.ct.cancelled().await;
                std::fs::write(self.root.join(format!("{}.cancelled", self.pid)), "")
                    .map_err(|_| ErrorData::internal_error("写测试事件失败", None))?;
                return Err(ErrorData::internal_error("已取消", None));
            }
            -2 => return Ok(CallToolResult::error(vec![]).into()),
            -3 => {
                return Ok(
                    CallToolResult::structured(json!({"value":"x".repeat(2*1024*1024)})).into(),
                );
            }
            -4 => return Ok(CallToolResult::structured(json!({"value":"invalid"})).into()),
            -5 => {
                while !self.root.join("release").exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
            -6 => std::future::pending::<()>().await,
            _ => {}
        }
        Ok(CallToolResult::structured(json!({"value":value * self.factor})).into())
    }
}
