#![cfg(feature = "server")]

use std::sync::Arc;

use anyhow::{Result, ensure};
use az_plugin_core::{DynPlugin, Plugin, PluginFuture};
use az_plugin_host::{
    configuration::HostConfig,
    generated::clipboard::{ClipboardService, model::*},
    identity::{IdentityProvider, SessionContext},
    runtime::server::{
        RuntimeState,
        http_error::RuntimeError,
        plugins::{
            ClipboardPlugin, DatabasePlugin, PostgresPlugin, RuntimeAssembly, StorePlugin,
            WorkerServicePlugin,
        },
    },
};

struct ReplacementClipboard;

#[async_trait::async_trait]
impl ClipboardService for ReplacementClipboard {
    async fn head(&self, owner: &Owner) -> Result<ClipHead, RuntimeError> {
        if owner.tenant != "plugin-test" || owner.user != "reader" {
            return Err(RuntimeError::forbidden("替代服务收到错误的调用身份"));
        }
        Ok(ClipHead {
            revision: 73,
            item: None,
        })
    }
    async fn list(&self, _: &Owner, _: Option<i64>, _: i64) -> Result<ClipPage, RuntimeError> {
        Err(RuntimeError::unavailable("夹具不提供条目分页"))
    }
    async fn read(&self, _: &Owner, _: &str) -> Result<ClipContent, RuntimeError> {
        Err(RuntimeError::unavailable("夹具不提供条目读取"))
    }
    async fn write(&self, _: &Owner, _: ClipWrite) -> Result<ClipItem, RuntimeError> {
        Err(RuntimeError::unavailable("夹具不提供条目写入"))
    }
    async fn devices(&self, _: &Owner) -> Result<Vec<ClipboardDevice>, RuntimeError> {
        Err(RuntimeError::unavailable("夹具不提供设备列表"))
    }
    async fn access(&self, _: &Owner, _: &str, _: bool) -> Result<(), RuntimeError> {
        Err(RuntimeError::unavailable("夹具不提供设备授权"))
    }
}

struct ReplacementPlugin;

impl Plugin<RuntimeAssembly> for ReplacementPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_value(ReplacementClipboard)
                .bind::<dyn ClipboardService, ReplacementClipboard>();
            Ok(())
        })
    }
}

struct Identity;

#[async_trait::async_trait]
impl IdentityProvider for Identity {
    async fn authenticate(
        &self,
        headers: &axum::http::HeaderMap,
    ) -> Result<Option<SessionContext>> {
        if headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            != Some("Bearer test-session")
        {
            return Ok(None);
        }
        Ok(Some(SessionContext {
            session_id: "test-session".into(),
            user_id: "reader".into(),
            account: "reader".into(),
            display_name: "Reader".into(),
            tenant_id: "plugin-test".into(),
            tenant_label: "Test".into(),
            permissions: vec![],
        }))
    }
    async fn can_publish(&self, _: &SessionContext) -> Result<bool> {
        Ok(false)
    }
    async fn member_active(&self, tenant: &str, user: &str) -> Result<bool> {
        Ok(tenant == "plugin-test" && user == "reader")
    }
    async fn session_active(&self, session: &str, tenant: &str, user: &str) -> Result<bool> {
        Ok(session == "test-session" && self.member_active(tenant, user).await?)
    }
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn external_plugin_replaces_builtin_service_without_replacing_authentication_or_routes()
-> Result<()> {
    let database = std::env::var("AIO_ENHANCEMENT_TEST_DATABASE_URL")?;
    ensure!(
        database.contains("127.0.0.1") && database.contains("_test"),
        "只接受本机独立测试库"
    );
    let root = tempfile::tempdir()?;
    let plugins: Vec<DynPlugin<RuntimeAssembly>> = vec![
        Arc::new(ClipboardPlugin),
        Arc::new(WorkerServicePlugin),
        Arc::new(StorePlugin),
        Arc::new(DatabasePlugin),
        Arc::new(PostgresPlugin),
        Arc::new(ReplacementPlugin),
    ];
    let state = RuntimeState::initialize_with_plugins(
        HostConfig {
            remote_capabilities: None,
            database_url: database,
            cache_root: root.path().join("cache"),
            public_origin: "http://127.0.0.1".into(),
            component_storage: None,
            default_plugins: vec![],
            delivery: None,
            development: None,
        },
        Arc::new(Identity),
        plugins,
    )
    .await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!(
        "http://{}/api/runtime/clipboard/head",
        listener.local_addr()?
    );
    let router = az_plugin_host::runtime::server::router(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let client = reqwest::Client::builder().no_proxy().build()?;
    let outcome = async {
        assert_eq!(client.get(&endpoint).send().await?.status(), 401);
        let response: serde_json::Value = client
            .get(&endpoint)
            .bearer_auth("test-session")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert_eq!(response["data"]["revision"], 73);
        Ok::<_, anyhow::Error>(())
    }
    .await;
    state.shutdown().await?;
    server.abort();
    outcome
}
