#![cfg(feature = "server")]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use anyhow::{Context, Result, ensure};
use az_plugin_contract::grpc::{InvokeRequest, enhancements_client::EnhancementsClient};
use az_plugin_core::{Plugin, PluginFuture};
use az_plugin_enhancement::{Language, Package, RhaiExecutor, ScriptContext};
use az_plugin_host::{
    configuration::HostConfig,
    identity::{IdentityProvider, SessionContext},
    runtime::server::{
        RuntimeState,
        plugins::{
            DatabasePlugin, EnhancementServicePlugin, EnhancementsPlugin, GrpcPlugin,
            PostgresPlugin, RuntimeAssembly, ScriptExecutor, ScriptProgram, ScriptRuntime,
            ScriptRuntimePlugin,
        },
    },
};
use serde_json::{Value, json};

struct Replacement(Arc<AtomicUsize>);

impl ScriptExecutor for Replacement {
    fn supports(&self, language: Language) -> bool {
        language == Language::Rhai
    }
    fn compile(&self, source: &str) -> Result<Arc<dyn ScriptProgram>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        RhaiExecutor.compile(source)
    }
}

struct ReplacementPlugin(Arc<AtomicUsize>);

impl Plugin<RuntimeAssembly> for ReplacementPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_value(Replacement(self.0.clone()))
                .bind::<dyn ScriptExecutor, Replacement>();
            Ok(())
        })
    }
}

struct Identity(String);

#[async_trait::async_trait]
impl IdentityProvider for Identity {
    async fn authenticate(
        &self,
        headers: &axum::http::HeaderMap,
    ) -> Result<Option<SessionContext>> {
        if headers
            .get("authorization")
            .and_then(|header| header.to_str().ok())
            != Some("Bearer executor-test")
        {
            return Ok(None);
        }
        Ok(Some(SessionContext {
            session_id: "executor-test".into(),
            user_id: "executor".into(),
            account: "executor".into(),
            display_name: "Executor".into(),
            tenant_id: self.0.clone(),
            tenant_label: "Test".into(),
            permissions: vec!["plugin:manage".into()],
        }))
    }
    async fn can_publish(&self, _: &SessionContext) -> Result<bool> {
        Ok(false)
    }
    async fn member_active(&self, tenant: &str, user: &str) -> Result<bool> {
        Ok(tenant == self.0 && user == "executor")
    }
    async fn session_active(&self, session: &str, tenant: &str, user: &str) -> Result<bool> {
        Ok(session == "executor-test" && self.member_active(tenant, user).await?)
    }
}

#[tokio::test]
async fn external_executor_uses_public_plugin_contract_without_database_or_component_storage()
-> Result<()> {
    let root = tempfile::tempdir()?;
    let calls = Arc::new(AtomicUsize::new(0));
    let state = RuntimeState::initialize_with_plugins(
        HostConfig {
            database_url: "deliberately invalid: no database".into(),
            public_origin: "http://127.0.0.1".into(),
            cache_root: root.path().join("cache"),
            component_storage: None,
            default_plugins: vec![],
            delivery: None,
            development: None,
        },
        Arc::new(Identity("unused".into())),
        vec![
            Arc::new(ScriptRuntimePlugin),
            Arc::new(ReplacementPlugin(calls.clone())),
        ],
    )
    .await?;
    let runtime = state.service::<ScriptRuntime>()?;
    let package = package()?;
    let prepared = runtime.prepare(package.clone())?;
    assert_eq!(
        prepared
            .invocation(
                ScriptContext::default(),
                vec!["/v1/math/double".into()],
                Default::default()
            )
            .invoke("/v1/math/double", &json!(21))?,
        json!(42)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let mut unsupported = package;
    unsupported.enhancements[0].language = Language::TypeScript;
    assert!(runtime.prepare(unsupported).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!root.path().join("cache").exists());
    state.shutdown().await?;
    Ok(())
}

fn package() -> Result<Package> {
    Ok(serde_json::from_value(json!({
        "schema_version": 1, "enhancements": [{
            "path": "/v1/math/double", "language": "rhai", "source": "fn invoke(input, context) { input * 2 }",
            "examples": [{"input": 3, "output": 6}]
        }]
    }))?)
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn external_executor_handles_publication_protocol_calls_and_persisted_restoration()
-> Result<()> {
    let database = std::env::var("AIO_ENHANCEMENT_TEST_DATABASE_URL")?;
    let url = reqwest::Url::parse(&database)?;
    ensure!(
        url.host_str() == Some("127.0.0.1") && url.path().ends_with("_test"),
        "只接受本机独立测试库"
    );
    let root = tempfile::tempdir()?;
    let identity = Arc::new(Identity(uuid::Uuid::new_v4().to_string()));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut revision = String::new();
    for generation in 0..2 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let state = RuntimeState::initialize_with_plugins(
            HostConfig {
                database_url: database.clone(),
                public_origin: base.clone(),
                cache_root: root.path().join("cache"),
                component_storage: None,
                default_plugins: vec![],
                delivery: None,
                development: None,
            },
            identity.clone(),
            vec![
                Arc::new(GrpcPlugin),
                Arc::new(EnhancementsPlugin),
                Arc::new(EnhancementServicePlugin),
                Arc::new(ScriptRuntimePlugin),
                Arc::new(ReplacementPlugin(calls.clone())),
                Arc::new(DatabasePlugin),
                Arc::new(PostgresPlugin),
            ],
        )
        .await?;
        let router = az_plugin_host::runtime::server::router(state.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await });
        let result = async {
            if generation == 0 {
                let client = reqwest::Client::builder().no_proxy().build()?;
                let proposed: Value = client
                    .post(format!("{base}/api/runtime/enhancements/revisions"))
                    .bearer_auth("executor-test")
                    .json(&package()?)
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                revision = proposed["revision"]
                    .as_str()
                    .context("缺少版本摘要")?
                    .into();
                client
                    .put(format!("{base}/api/runtime/enhancements/active"))
                    .bearer_auth("executor-test")
                    .json(&json!({"revision":revision,"expected_generation":0}))
                    .send()
                    .await?
                    .error_for_status()?;
            }
            let mut client = EnhancementsClient::connect(base.clone()).await?;
            let mut request = tonic::Request::new(InvokeRequest {
                path: "/v1/math/double".into(),
                input_json: serde_json::to_vec(&json!(21))?,
            });
            request
                .metadata_mut()
                .insert("authorization", "Bearer executor-test".parse()?);
            let response = client.invoke(request).await?.into_inner();
            assert_eq!(response.revision, revision);
            assert_eq!(
                serde_json::from_slice::<Value>(&response.output_json)?,
                json!(42)
            );
            // 首次发布与重建后的懒恢复都必须走应用提供的执行器。
            assert_eq!(calls.load(Ordering::SeqCst), generation + 1);
            assert!(!root.path().join("cache").exists());
            Ok::<_, anyhow::Error>(())
        }
        .await;
        let cleanup = state.shutdown().await;
        server.abort();
        let _ = server.await;
        result?;
        cleanup?;
    }
    Ok(())
}
