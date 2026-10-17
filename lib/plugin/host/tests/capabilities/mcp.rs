use az_plugin_core::{Plugin, PluginFuture};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, PaginatedRequestParams},
    service::RunningService,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};

use super::*;

async fn client(base: &str, actor: &str) -> Result<RunningService<RoleClient, ()>> {
    let config = StreamableHttpClientTransportConfig::with_uri(format!("{base}/api/runtime/mcp"))
        .auth_header(actor);
    let transport = StreamableHttpClientTransport::with_client(reqwest_mcp::Client::new(), config);
    Ok(().serve(transport).await?)
}

fn input(value: Value) -> CallToolRequestParams {
    CallToolRequestParams::new("runtime.v1.native.execute")
        .with_arguments(serde_json::Map::from_iter([("input".into(), value)]))
}

#[tokio::test]
async fn mcp_disconnect_cancels_native_future() -> Result<()> {
    let fixture = Fixture::open(Identity::default(), vec![]).await?;
    let request = reqwest::Client::builder()
        .no_proxy()
        .build()?
        .post(format!("{}/api/runtime/mcp", fixture.base))
        .bearer_auth("capable")
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-06-18")
        .json(&json!({"jsonrpc":"2.0", "id":1, "method":"tools/call",
            "params":{"name":"runtime.v1.native.execute","arguments":{"input":"wait"}}}));
    let pending = tokio::spawn(async move { request.send().await });
    tokio::time::timeout(Duration::from_secs(3), fixture.probe.entered.notified()).await?;
    assert_eq!(fixture.probe.active.load(Ordering::SeqCst), 1);
    pending.abort();
    assert!(
        pending
            .await
            .err()
            .context("请求没有被取消")?
            .is_cancelled()
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.probe.active.load(Ordering::SeqCst) != 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .context("MCP 断连未取消原生调用")?;
    fixture.close().await
}

#[tokio::test]
async fn mcp_native_tools_enforce_permissions_contracts_and_live_replacement() -> Result<()> {
    let mut fixture = Fixture::open(Identity::default(), vec![]).await?;
    for actor in ["limited", "manager", "other"] {
        let denied = client(&fixture.base, actor).await?;
        assert!(denied.list_all_tools().await?.is_empty());
        assert_eq!(
            denied.call_tool(input(Value::Null)).await?.is_error,
            Some(true)
        );
        denied.cancel().await?;
    }
    assert_eq!(fixture.probe.calls.load(Ordering::SeqCst), 0);
    let client = client(&fixture.base, "capable").await?;
    let mut enhancement = input(Value::Null);
    enhancement.name = "v1.native.execute".into();
    assert_eq!(client.call_tool(enhancement).await?.is_error, Some(true));
    let tools = client.list_all_tools().await?;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "runtime.v1.native.execute");
    let annotations = tools[0].annotations.as_ref().context("缺少副作用声明")?;
    assert_eq!(annotations.read_only_hint, Some(false));
    assert_eq!(annotations.idempotent_hint, Some(false));
    let result = client.call_tool(input(json!(21))).await?;
    let result = result.structured_content.context("缺少输出")?;
    assert_eq!(result, json!({"output":42}));
    let output_schema = tools[0].output_schema.as_ref().context("缺少输出契约")?;
    let validator = jsonschema::options()
        .offline()
        .build(&json!(output_schema))?;
    assert!(validator.is_valid(&result));
    assert!(!validator.is_valid(&json!({"output":"wrong"})));
    fixture.assert_scope()?;
    for value in [
        json!({"tenant_id":"other"}),
        json!("invalid"),
        json!("large"),
        json!("fail"),
    ] {
        assert_eq!(client.call_tool(input(value)).await?.is_error, Some(true));
    }
    assert_eq!(fixture.probe.calls.load(Ordering::SeqCst), 4);
    let mut forged = input(Value::Null);
    forged
        .arguments
        .as_mut()
        .context("缺少参数")?
        .insert("scope".into(), json!({"grants":{"runtime":[PATH]}}));
    assert!(client.call_tool(forged).await.is_err());
    assert_eq!(fixture.probe.calls.load(Ordering::SeqCst), 4);
    fixture
        .identity
        .runtime_revoked
        .store(true, Ordering::SeqCst);
    assert!(client.list_all_tools().await?.is_empty());
    assert_eq!(
        client.call_tool(input(Value::Null)).await?.is_error,
        Some(true)
    );
    fixture
        .identity
        .runtime_revoked
        .store(false, Ordering::SeqCst);
    fixture.identity.revoked.store(true, Ordering::SeqCst);
    assert!(client.list_all_tools().await.is_err());
    fixture.identity.revoked.store(false, Ordering::SeqCst);
    fixture.plugins[1] = Arc::new(ProviderPlugin(fixture.probe.clone(), 84));
    fixture.host.reconfigure(fixture.plugins.clone()).await?;
    assert_eq!(
        client
            .call_tool(input(Value::Null))
            .await?
            .structured_content,
        Some(json!({"output":84}))
    );
    fixture
        .plugins
        .retain(|plugin| plugin.as_ref().type_id() != TypeId::of::<McpPlugin>());
    fixture.host.reconfigure(fixture.plugins.clone()).await?;
    assert!(client.list_all_tools().await.is_err());
    fixture.plugins.push(Arc::new(McpPlugin));
    fixture.host.reconfigure(fixture.plugins.clone()).await?;
    assert_eq!(client.list_all_tools().await?.len(), 1);
    client.cancel().await?;
    fixture.close().await
}

struct Catalog;
#[async_trait::async_trait]
impl CapabilityProvider for Catalog {
    fn capabilities(&self) -> Vec<RuntimeCapability> {
        (0..150)
            .map(|index| RuntimeCapability {
                path: format!("/v1/catalog/item-{index:03}"),
                description: "分页测试".into(),
                effect: CapabilityEffect::Read,
                input_schema: serde_json::Map::from_iter([
                    ("type".into(), json!("integer")),
                    ("description".into(), json!("x".repeat(9000))),
                ])
                .into(),
                output_schema: schemars::schema_for!(i64),
            })
            .collect()
    }
    async fn invoke(&self, _: &CapabilityRequest) -> Result<Value> {
        Ok(json!(1))
    }
}
impl Plugin<RuntimeAssembly> for Catalog {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_value(Catalog)
                .bind::<dyn CapabilityProvider, Catalog>();
            Ok(())
        })
    }
}

#[tokio::test]
async fn mcp_large_catalog_pages_are_bounded_and_recheck_revocation() -> Result<()> {
    let identity = Identity {
        additional_paths: (0..150)
            .map(|index| format!("/v1/catalog/item-{index:03}"))
            .collect(),
        ..Default::default()
    };
    let fixture = Fixture::open(identity, vec![Arc::new(Catalog)]).await?;
    let client = client(&fixture.base, "capable").await?;
    let first = client.list_tools(None).await?;
    assert!(first.tools.len() < 64);
    assert!(serde_json::to_vec(&first)?.len() <= 256 * 1024);
    let cursor = first.next_cursor.context("缺少下一页")?;
    let mut all = first.tools;
    let mut next = Some(cursor.clone());
    while let Some(cursor) = next {
        let page = client
            .list_tools(Some(
                PaginatedRequestParams::default().with_cursor(Some(cursor)),
            ))
            .await?;
        assert!(serde_json::to_vec(&page)?.len() <= 256 * 1024);
        all.extend(page.tools);
        next = page.next_cursor;
    }
    assert_eq!(all.len(), 151);
    assert!(all.windows(2).all(|pair| pair[0].name < pair[1].name));
    fixture
        .identity
        .runtime_revoked
        .store(true, Ordering::SeqCst);
    let page = client
        .list_tools(Some(
            PaginatedRequestParams::default().with_cursor(Some(cursor)),
        ))
        .await?;
    assert!(page.tools.is_empty());
    assert!(page.next_cursor.is_none());
    for cursor in ["invalid!", &"x".repeat(300)] {
        assert!(
            client
                .list_tools(Some(
                    PaginatedRequestParams::default().with_cursor(Some(cursor.into()))
                ))
                .await
                .is_err()
        );
    }
    client.cancel().await?;
    fixture.close().await
}
