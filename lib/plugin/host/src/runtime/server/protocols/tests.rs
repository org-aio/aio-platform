use std::sync::atomic::Ordering;

use anyhow::{Context, Result};
use az_plugin_contract::grpc::{
    InvokeRequest, ListRequest, enhancements_client::EnhancementsClient,
};
use az_plugin_enhancement::Language;
use rmcp::model::CallToolRequestParams;
use serde_json::json;
use tonic::Code;

#[path = "test_support.rs"]
mod support;
use support::*;

#[path = "capability_tests.rs"]
mod capabilities;
#[path = "composition_tests.rs"]
mod composition;
#[path = "evolution_tests.rs"]
mod evolution;
#[cfg(feature = "remote")]
#[path = "remote_tests.rs"]
mod remote;

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn real_protocol_clients_share_updates_permissions_and_revocation() -> Result<()> {
    let fixture = Fixture::open().await?;
    let outcome =
        tokio::time::timeout(std::time::Duration::from_secs(60), exercise(&fixture)).await;
    fixture.server.abort();
    outcome??;
    Ok(())
}

async fn exercise(fixture: &Fixture) -> Result<()> {
    let first = fixture.publish(package(Language::Rhai), 0).await?;
    let mcp = mcp_client(&fixture.base, "limited").await?;
    let manager = mcp_client(&fixture.base, "manager").await?;
    let denied = mcp_client(&fixture.base, "denied").await?;
    let other = mcp_client(&fixture.base, "other").await?;
    let mut grpc = EnhancementsClient::connect(fixture.base.clone()).await?;

    let tools = mcp.list_all_tools().await?;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "v1.math.double");
    assert_eq!(manager.list_all_tools().await?.len(), 2);
    assert!(denied.list_all_tools().await?.is_empty());
    assert!(other.list_all_tools().await?.is_empty());
    let catalog = grpc
        .list(auth(ListRequest {}, "limited")?)
        .await?
        .into_inner();
    assert_eq!(catalog.paths, [PATH]);
    assert_eq!(catalog.revision.as_deref(), Some(first.as_str()));
    assert!(
        grpc.list(auth(ListRequest {}, "other")?)
            .await?
            .into_inner()
            .revision
            .is_none()
    );
    assert_eq!(
        grpc.list(ListRequest {})
            .await
            .err()
            .context("匿名请求被接受")?
            .code(),
        Code::Unauthenticated
    );
    assert!(mcp_client(&fixture.base, "unknown").await.is_err());
    assert_eq!(
        grpc.invoke(auth(invoke(), "denied")?)
            .await
            .err()
            .context("越权调用被接受")?
            .code(),
        Code::PermissionDenied
    );
    assert_eq!(
        grpc.invoke(auth(invoke(), "other")?)
            .await
            .err()
            .context("跨租户调用被接受")?
            .code(),
        Code::FailedPrecondition
    );
    let denied_result = denied.call_tool(call()).await?;
    assert_eq!(denied_result.is_error, Some(true));
    let mut invalid = invoke();
    invalid.input_json = b"not json".to_vec();
    assert_eq!(
        grpc.invoke(auth(invalid, "manager")?)
            .await
            .err()
            .context("无效 JSON 被接受")?
            .code(),
        Code::InvalidArgument
    );

    compare(&mcp, &mut grpc, &first).await?;
    let scope = grpc
        .invoke(auth(
            InvokeRequest {
                path: "/v1/caller/scope".into(),
                input_json: serde_json::to_vec(
                    &json!({"tenant_id":"forged","source_id":"forged"}),
                )?,
            },
            "manager",
        )?)
        .await?
        .into_inner();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&scope.output_json)?,
        json!({"tenant":fixture.identity.tenant,"source":"user:manager"})
    );

    // 保持原来的连接，验证新脚本激活与回滚无需重建协议服务或客户端。
    let mut upgraded = package(Language::TypeScript);
    upgraded
        .enhancements
        .push(az_plugin_enhancement::Enhancement {
            capabilities: vec![],
            calls: vec![],
            path: "/v1/math/triple".into(),
            language: Language::TypeScript,
            source: "export function invoke(input: number): number { return input * 3; }".into(),
            examples: vec![az_plugin_enhancement::Example {
                capabilities: vec![],
                input: json!(3),
                output: json!(9),
            }],
        });
    let second = fixture.publish(upgraded, 1).await?;
    assert_ne!(first, second);
    compare(&mcp, &mut grpc, &second).await?;
    assert_eq!(manager.list_all_tools().await?.len(), 3);
    assert_eq!(mcp.list_all_tools().await?.len(), 1);
    assert_eq!(
        grpc.list(auth(ListRequest {}, "manager")?)
            .await?
            .into_inner()
            .paths
            .len(),
        3
    );
    reqwest::Client::new()
        .post(format!(
            "{}/api/runtime/enhancements/rollback",
            fixture.base
        ))
        .bearer_auth("manager")
        .json(&json!({"revision":first,"expected_generation":2}))
        .send()
        .await?
        .error_for_status()?;
    compare(&mcp, &mut grpc, &first).await?;

    fixture.identity.revoked.store(true, Ordering::SeqCst);
    assert!(mcp.list_all_tools().await.is_err());
    assert_eq!(
        grpc.invoke(auth(invoke(), "limited")?)
            .await
            .err()
            .context("已撤销会话仍可调用")?
            .code(),
        Code::Unauthenticated
    );
    assert_eq!(manager.list_all_tools().await?.len(), 2);

    transport_limits(fixture, &mut grpc).await?;
    fixture
        .state
        .service::<super::super::enhancements::Enhancements>()?
        .activate(
            &fixture.identity.tenant,
            "test",
            crate::runtime::server::enhancements::Activation {
                revision: None,
                expected_generation: 3,
            },
            false,
            async { Ok(()) },
        )
        .await?
        .context("停用失败")?;
    assert!(manager.list_all_tools().await?.is_empty());
    assert!(
        grpc.list(auth(ListRequest {}, "manager")?)
            .await?
            .into_inner()
            .paths
            .is_empty()
    );
    assert_eq!(manager.call_tool(call()).await?.is_error, Some(true));
    assert_eq!(
        grpc.invoke(auth(invoke(), "manager")?)
            .await
            .err()
            .context("停用后仍可调用")?
            .code(),
        Code::FailedPrecondition
    );
    mcp.cancel().await?;
    manager.cancel().await?;
    denied.cancel().await?;
    other.cancel().await?;
    fixture.check_omitted_plugins().await?;
    fixture.state.shutdown().await?;
    assert!(fixture.state.service::<sqlx::PgPool>()?.is_closed());
    assert_eq!(
        grpc.list(auth(ListRequest {}, "manager")?)
            .await
            .err()
            .context("关闭后仍可调用 gRPC")?
            .code(),
        Code::Unavailable
    );
    assert_eq!(
        reqwest::Client::new()
            .post(format!("{}/api/runtime/mcp", fixture.base))
            .send()
            .await?
            .status(),
        503
    );
    Ok(())
}

fn invoke() -> InvokeRequest {
    InvokeRequest {
        path: PATH.into(),
        input_json: b"21".to_vec(),
    }
}

fn call() -> CallToolRequestParams {
    CallToolRequestParams::new("v1.math.double")
        .with_arguments(serde_json::Map::from_iter([("input".into(), json!(21))]))
}

async fn compare(
    mcp: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    grpc: &mut EnhancementsClient<tonic::transport::Channel>,
    revision: &str,
) -> Result<()> {
    let mcp_result = mcp.call_tool(call()).await?;
    assert_ne!(mcp_result.is_error, Some(true));
    assert_eq!(
        mcp_result.structured_content,
        Some(json!({"revision":revision,"output":42}))
    );
    let grpc_result = grpc.invoke(auth(invoke(), "limited")?).await?.into_inner();
    assert_eq!(grpc_result.revision, revision);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&grpc_result.output_json)?,
        json!(42)
    );
    Ok(())
}

async fn transport_limits(
    fixture: &Fixture,
    grpc: &mut EnhancementsClient<tonic::transport::Channel>,
) -> Result<()> {
    let client = reqwest::Client::new();
    assert_eq!(
        client
            .get(format!("{}/absent", fixture.base))
            .bearer_auth("manager")
            .send()
            .await?
            .status(),
        404
    );
    let url = format!("{}/api/runtime/mcp", fixture.base);
    let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/list"});
    for (header, value) in [
        ("origin", "https://untrusted.invalid"),
        ("host", "untrusted.invalid"),
    ] {
        assert_eq!(
            client
                .post(&url)
                .bearer_auth("manager")
                .header(header, value)
                .header("accept", "application/json, text/event-stream")
                .json(&request)
                .send()
                .await?
                .status(),
            403
        );
    }
    let chunks =
        futures_util::stream::iter((0..3).map(|_| Ok::<_, std::io::Error>(vec![b'x'; 64 * 1024])));
    assert_eq!(
        client
            .post(&url)
            .bearer_auth("manager")
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(reqwest::Body::wrap_stream(chunks))
            .send()
            .await?
            .status(),
        413
    );
    let mut oversized = invoke();
    oversized.input_json = vec![b'x'; 128 * 1024];
    let error = grpc
        .invoke(auth(oversized, "manager")?)
        .await
        .err()
        .context("超限消息被接受")?;
    assert_eq!(error.code(), Code::OutOfRange);
    // Tonic 客户端会先自行取消过期请求；用真实 HTTP/2 观察宿主的截止时间响应。
    let wire = reqwest::Client::builder()
        .http2_prior_knowledge()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    for (deadline, actor, status) in [
        ("0n", "manager", "4"),
        ("1m", "slow", "4"),
        ("invalid", "manager", "3"),
    ] {
        let response = wire
            .post(format!("{}/aio.runtime.v1.Enhancements/List", fixture.base))
            .bearer_auth(actor)
            .header("content-type", "application/grpc")
            .header("te", "trailers")
            .header("grpc-timeout", deadline)
            // 空 ListRequest 的标准 gRPC 帧：无压缩标记和四字节零长度。
            .body(vec![0_u8; 5])
            .send()
            .await?;
        assert_eq!(response.version(), reqwest::Version::HTTP_2);
        assert_eq!(
            response
                .headers()
                .get("grpc-status")
                .context("宿主未返回 gRPC 错误码")?,
            status
        );
    }
    Ok(())
}
