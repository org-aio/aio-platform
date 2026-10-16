//! 保持真实 MCP/gRPC 客户端连接，在线重建语言与数据库插件，并卸载、恢复协议入口。

use anyhow::{Context, Result, ensure};
use az_plugin_contract::grpc::enhancements_client::EnhancementsClient;
use az_plugin_core::DynPlugin;
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
            Arc::new(EnhancementServicePlugin),
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
        compare(&mcp, &mut grpc, &revision).await?;
        host.restart::<RhaiPlugin>().await?;
        compare(&mcp, &mut grpc, &revision).await?;
        host.restart::<DatabasePlugin>().await?;
        compare(&mcp, &mut grpc, &revision).await?;
        let revision = publish(&base, package(Language::TypeScript), 1).await?;
        host.restart::<TypeScriptPlugin>().await?;
        compare(&mcp, &mut grpc, &revision).await?;
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
        Ok::<_, anyhow::Error>(())
    })
    .await;
    let cleanup = host.shutdown().await;
    server.abort();
    result??;
    cleanup
}
