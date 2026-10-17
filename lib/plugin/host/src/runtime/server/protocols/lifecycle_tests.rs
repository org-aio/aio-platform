//! 保持真实 MCP/gRPC 客户端连接，在线重建语言与数据库插件，并卸载、恢复协议入口。

use anyhow::{Context, Result, ensure};
use az_plugin_contract::grpc::{
    InvokeRequest, capabilities_client::CapabilitiesClient, enhancements_client::EnhancementsClient,
};
use az_plugin_core::{DynPlugin, Plugin, PluginFuture};
use az_plugin_enhancement::Language;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use super::{
    compare,
    support::{Identity, mcp_client, package, publish},
};
use crate::runtime::server::{admin_test_support::with_identity, plugins::*};

struct Native;

#[async_trait::async_trait]
impl CapabilityProvider for Native {
    fn capabilities(&self) -> Vec<RuntimeCapability> {
        vec![RuntimeCapability {
            path: "/v1/remote/mcp".into(),
            description: "原生生命周期探针".into(),
            effect: CapabilityEffect::Read,
            input_schema: schemars::schema_for!(i64),
            output_schema: schemars::schema_for!(i64),
        }]
    }
    async fn invoke(&self, request: &CapabilityRequest) -> Result<serde_json::Value> {
        assert_eq!(request.scope().source_id, "user:remote");
        let value = request.input.as_i64().context("输入不是整数")?;
        Ok(serde_json::json!(value.checked_mul(2).context("整数溢出")?))
    }
}

impl Plugin<RuntimeAssembly> for Native {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_value(Native)
                .bind::<dyn CapabilityProvider, Native>();
            Ok(())
        })
    }
}

async fn native(
    mcp: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    grpc: &mut CapabilitiesClient<tonic::transport::Channel>,
) -> Result<()> {
    let tools = mcp.list_all_tools().await?;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "runtime.v1.remote.mcp");
    let output = mcp
        .call_tool(
            rmcp::model::CallToolRequestParams::new("runtime.v1.remote.mcp").with_arguments(
                serde_json::Map::from_iter([("input".into(), serde_json::json!(21))]),
            ),
        )
        .await?;
    assert_eq!(
        output.structured_content,
        Some(serde_json::json!({"output":42}))
    );
    let output = grpc
        .invoke(super::auth(
            InvokeRequest {
                path: "/v1/remote/mcp".into(),
                input_json: b"21".to_vec(),
            },
            "remote",
        )?)
        .await?
        .into_inner();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.output_json)?,
        42
    );
    Ok(())
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn clients_survive_native_executor_database_restart_and_protocol_unload() -> Result<()> {
    let database = std::env::var("AIO_ENHANCEMENT_TEST_DATABASE_URL")?;
    ensure!(
        database.contains("127.0.0.1") && database.contains("_test"),
        "只接受本机独立测试库"
    );
    let root = tempfile::tempdir()?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let identity = Arc::new(Identity {
        tenant: uuid::Uuid::new_v4().to_string(),
        revoked: AtomicBool::new(false),
        runtime_allowed: AtomicBool::new(true),
        management_allowed: AtomicBool::new(true),
    });
    let plugins: Vec<DynPlugin<RuntimeAssembly>> = with_identity(
        identity.clone(),
        vec![
            Arc::new(McpPlugin),
            Arc::new(GrpcPlugin),
            Arc::new(EnhancementsPlugin),
            Arc::new(PostgresEnhancementCatalogPlugin),
            Arc::new(EnhancementServicePlugin),
            Arc::new(CapabilityServicePlugin),
            Arc::new(Native),
            Arc::new(ScriptRuntimePlugin),
            Arc::new(RhaiPlugin),
            Arc::new(TypeScriptPlugin),
            Arc::new(DatabasePlugin),
            Arc::new(PostgresPlugin),
        ],
    );
    let host = RuntimeHost::initialize(
        crate::configuration::HostConfig {
            remote_capabilities: None,
            database_url: database,
            public_origin: base.clone(),
            cache_root: root.path().join("cache"),
            component_storage: None,
            default_plugins: vec![],
            delivery: None,
            development: None,
        },
        plugins.clone(),
    )
    .await?;
    let router = host.router();
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let result = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        let revision = publish(&base, package(Language::Rhai), 0).await?;
        let mcp = mcp_client(&base, "limited").await?;
        let mut grpc = EnhancementsClient::connect(base.clone()).await?;
        let native_mcp = mcp_client(&base, "remote").await?;
        let mut native_grpc = CapabilitiesClient::connect(base.clone()).await?;
        native(&native_mcp, &mut native_grpc).await?;
        compare(&mcp, &mut grpc, &revision).await?;
        host.restart::<RhaiPlugin>().await?;
        compare(&mcp, &mut grpc, &revision).await?;
        host.restart::<DatabasePlugin>().await?;
        compare(&mcp, &mut grpc, &revision).await?;
        let revision = publish(&base, package(Language::TypeScript), 1).await?;
        host.restart::<TypeScriptPlugin>().await?;
        compare(&mcp, &mut grpc, &revision).await?;
        // 在线卸载解释器、版本服务和数据库后，同一连接仍能调用原生能力。
        let removed = [
            std::any::TypeId::of::<EnhancementsPlugin>(),
            std::any::TypeId::of::<EnhancementServicePlugin>(),
            std::any::TypeId::of::<PostgresEnhancementCatalogPlugin>(),
            std::any::TypeId::of::<ScriptRuntimePlugin>(),
            std::any::TypeId::of::<RhaiPlugin>(),
            std::any::TypeId::of::<TypeScriptPlugin>(),
            std::any::TypeId::of::<DatabasePlugin>(),
            std::any::TypeId::of::<PostgresPlugin>(),
        ];
        host.reconfigure(
            plugins
                .iter()
                .filter(|plugin| !removed.contains(&plugin.as_ref().type_id()))
                .cloned()
                .collect(),
        )
        .await?;
        native(&native_mcp, &mut native_grpc).await?;
        assert!(mcp.list_all_tools().await?.is_empty());
        assert!(
            grpc.invoke(super::auth(super::invoke(), "limited")?)
                .await
                .is_err()
        );
        host.reconfigure(plugins.clone()).await?;
        compare(&mcp, &mut grpc, &revision).await?;
        native(&native_mcp, &mut native_grpc).await?;
        // 混合包是一个执行单元；移除任一语言后原生能力仍可发现与调用。
        let mut mixed = package(Language::Rhai);
        let mut extra = package(Language::TypeScript).enhancements.remove(0);
        extra.path = "/v1/math/base".into();
        mixed.enhancements.push(extra);
        let revision = publish(&base, mixed, 2).await?;
        for removed in [
            std::any::TypeId::of::<RhaiPlugin>(),
            std::any::TypeId::of::<TypeScriptPlugin>(),
        ] {
            host.reconfigure(
                plugins
                    .iter()
                    .filter(|plugin| plugin.as_ref().type_id() != removed)
                    .cloned()
                    .collect(),
            )
            .await?;
            native(&native_mcp, &mut native_grpc).await?;
            assert!(mcp.list_all_tools().await?.is_empty());
            let catalog = grpc
                .list(super::auth(
                    az_plugin_contract::grpc::ListRequest {},
                    "limited",
                )?)
                .await?
                .into_inner();
            assert_eq!(catalog.revision.as_deref(), Some(revision.as_str()));
            assert!(catalog.paths.is_empty());
            assert_eq!(
                grpc.invoke(super::auth(super::invoke(), "limited")?)
                    .await
                    .err()
                    .context("混合包仍在执行")?
                    .code(),
                tonic::Code::PermissionDenied
            );
            host.reconfigure(plugins.clone()).await?;
            compare(&mcp, &mut grpc, &revision).await?;
        }
        let without_mcp = plugins
            .iter()
            .filter(|plugin| plugin.as_ref().type_id() != std::any::TypeId::of::<McpPlugin>())
            .cloned()
            .collect();
        host.reconfigure(without_mcp).await?;
        assert_eq!(
            reqwest::Client::new()
                .post(format!("{base}/api/runtime/mcp"))
                .send()
                .await?
                .status(),
            404
        );
        assert_eq!(
            grpc.invoke(super::auth(super::invoke(), "limited")?)
                .await?
                .into_inner()
                .revision,
            revision
        );
        host.reconfigure(plugins).await?;
        compare(&mcp, &mut grpc, &revision).await?;
        identity.revoked.store(true, Ordering::SeqCst);
        assert!(mcp.list_all_tools().await.is_err());
        assert_eq!(
            grpc.invoke(super::auth(super::invoke(), "limited")?)
                .await
                .err()
                .context("重启丢失撤权检查")?
                .code(),
            tonic::Code::Unauthenticated
        );
        mcp.cancel().await?;
        native_mcp.cancel().await?;
        Ok::<_, anyhow::Error>(())
    })
    .await;
    let cleanup = host.shutdown().await;
    server.abort();
    result??;
    cleanup
}
