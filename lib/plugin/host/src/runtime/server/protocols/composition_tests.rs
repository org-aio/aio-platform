use std::sync::atomic::Ordering;

use anyhow::{Context, Result};
use az_plugin_contract::{
    EnhancementRequest, InvocationScope,
    grpc::{InvokeRequest, ListRequest, enhancements_client::EnhancementsClient},
};
use az_plugin_enhancement::Package;
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};
use tonic::Code;

use super::support::*;
use crate::runtime::server::enhancements::Enhancements;

fn package() -> Result<Package> {
    Ok(serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../enhancement/examples/composition.json"
    )))?)
}

fn call(path: &str) -> CallToolRequestParams {
    CallToolRequestParams::new(path.trim_start_matches('/').replace('/', "."))
        .with_arguments(serde_json::Map::from_iter([("input".into(), json!(21))]))
}

fn rpc(path: &str) -> InvokeRequest {
    InvokeRequest {
        path: path.into(),
        input_json: b"21".to_vec(),
    }
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn real_protocols_enforce_transitive_grants_restore_and_rollback_composed_scripts()
-> Result<()> {
    let fixture = Fixture::open().await?;
    let outcome =
        tokio::time::timeout(std::time::Duration::from_secs(60), exercise(&fixture)).await;
    let shutdown = fixture.state.shutdown().await;
    fixture.server.abort();
    outcome??;
    shutdown?;
    Ok(())
}

async fn exercise(fixture: &Fixture) -> Result<()> {
    let first = fixture.publish(package()?, 0).await?;
    let mcp = mcp_client(&fixture.base, "composed").await?;
    let limited = mcp_client(&fixture.base, "limited").await?;
    let manager = mcp_client(&fixture.base, "manager").await?;
    let mut grpc = EnhancementsClient::connect(fixture.base.clone()).await?;
    assert_eq!(mcp.list_all_tools().await?.len(), 3);
    assert!(limited.list_all_tools().await?.is_empty());
    assert!(
        grpc.list(auth(ListRequest {}, "limited")?)
            .await?
            .into_inner()
            .paths
            .is_empty()
    );
    assert_eq!(limited.call_tool(call(PATH)).await?.is_error, Some(true));
    assert_eq!(
        grpc.invoke(auth(rpc(PATH), "limited")?)
            .await
            .err()
            .context("间接越权被接受")?
            .code(),
        Code::PermissionDenied
    );
    let pipeline = "/v1/math/pipeline";
    let mut changed = package()?;
    changed.enhancements[0].source =
        "fn invoke(input, context) { if input == 3 { 6 } else { input * 3 } }".into();
    compare(&mcp, &mut grpc, &first, 43).await?;
    let second = fixture.publish(changed, 1).await?;
    compare(&mcp, &mut grpc, &second, 64).await?;
    let service = fixture.state.service::<Enhancements>()?;
    let head = service.head(&fixture.identity.tenant).await?;
    reqwest::Client::new()
        .post(format!(
            "{}/api/runtime/enhancements/rollback",
            fixture.base
        ))
        .bearer_auth("manager")
        .json(&json!({"revision":first,"expected_generation":head.generation}))
        .send()
        .await?
        .error_for_status()?;
    assert_eq!(
        manager.call_tool(call(pipeline)).await?.structured_content,
        Some(json!({"revision":first,"output":43}))
    );

    // 新服务从 PostgreSQL 恢复相同依赖声明与版本，不能依赖先前编译缓存。
    let restored = Enhancements::open(
        fixture.state.service::<sqlx::PgPool>()?.as_ref().clone(),
        az_plugin_enhancement::default_runtime()?,
    )
    .await?;
    let scope = InvocationScope {
        source_id: "restored-plugin".into(),
        revision: "plugin-version".into(),
        context: az_plugin_contract::RequestContext {
            tenant_id: Some(fixture.identity.tenant.clone()),
            ..Default::default()
        },
        grants: az_plugin_contract::CapabilityGrants {
            enhancements: vec![PATH.into(), "/v1/math/base".into(), pipeline.into()],
            ..Default::default()
        },
    };
    let output = restored
        .invoke(
            &scope,
            EnhancementRequest {
                path: pipeline.into(),
                input: json!(21),
            },
        )
        .await?;
    assert_eq!(output.revision, first);
    assert_eq!(output.output, json!(43));
    let mut broken = package()?;
    broken.enhancements[0].calls.push(pipeline.into());
    assert!(
        fixture
            .publish(
                broken,
                restored.head(&fixture.identity.tenant).await?.generation
            )
            .await
            .is_err()
    );
    assert_eq!(
        restored
            .head(&fixture.identity.tenant)
            .await?
            .revision
            .as_deref(),
        Some(first.as_str())
    );
    fixture.identity.revoked.store(true, Ordering::SeqCst);
    assert!(mcp.list_all_tools().await.is_err());
    assert_eq!(
        grpc.invoke(auth(rpc(pipeline), "composed")?)
            .await
            .err()
            .context("撤权未生效")?
            .code(),
        Code::Unauthenticated
    );
    mcp.cancel().await?;
    limited.cancel().await?;
    manager.cancel().await?;
    Ok(())
}

async fn compare(
    mcp: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    grpc: &mut EnhancementsClient<tonic::transport::Channel>,
    revision: &str,
    expected: i64,
) -> Result<()> {
    let result = mcp.call_tool(call("/v1/math/pipeline")).await?;
    assert_eq!(
        result.structured_content,
        Some(json!({"revision":revision,"output":expected}))
    );
    let result = grpc
        .invoke(auth(rpc("/v1/math/pipeline"), "composed")?)
        .await?
        .into_inner();
    assert_eq!(result.revision, revision);
    assert_eq!(
        serde_json::from_slice::<Value>(&result.output_json)?,
        json!(expected)
    );
    Ok(())
}
