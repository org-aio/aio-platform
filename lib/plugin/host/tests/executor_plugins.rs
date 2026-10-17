#![cfg(feature = "server")]

#[cfg(any(feature = "mcp", feature = "grpc"))]
#[path = "support/identity.rs"]
mod identity_support;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[cfg(any(feature = "mcp", feature = "grpc"))]
use anyhow::ensure;
use anyhow::{Context, Result};
#[cfg(feature = "grpc")]
use az_plugin_contract::grpc::{InvokeRequest, enhancements_client::EnhancementsClient};
use az_plugin_core::{Plugin, PluginFuture};
use az_plugin_enhancement::{Language, Package, ScriptContext};
#[cfg(feature = "grpc")]
use az_plugin_host::runtime::server::plugins::GrpcPlugin;
#[cfg(feature = "mcp")]
use az_plugin_host::runtime::server::plugins::McpPlugin;
use az_plugin_host::{
    configuration::HostConfig,
    runtime::server::{
        RuntimeState,
        plugins::{
            RuntimeAssembly, ScriptExecutor, ScriptInvocation, ScriptProgram, ScriptRuntime,
            ScriptRuntimePlugin,
        },
    },
};
#[cfg(any(feature = "mcp", feature = "grpc"))]
use az_plugin_host::{
    identity::{IdentityProvider, SessionContext},
    runtime::server::plugins::{
        CapabilityServicePlugin, DatabasePlugin, EnhancementServicePlugin, EnhancementsPlugin,
        PostgresPlugin,
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
        // 独立的有限语法测试实现，用于证明公共契约不依赖任何内置解释器。
        let factor = source
            .strip_prefix("fn invoke(input, context) { input * ")
            .and_then(|source| source.strip_suffix(" }"))
            .context("测试执行器只支持整数乘法")?
            .parse()?;
        Ok(Arc::new(Multiplier(factor)))
    }
}

struct Multiplier(i64);

impl ScriptProgram for Multiplier {
    fn invoke(&self, input: &Value, invocation: &ScriptInvocation) -> Result<Value> {
        invocation.check()?;
        let value = input.as_i64().context("输入必须是整数")?;
        let output = value.checked_mul(self.0).context("整数乘法溢出")?;
        Ok(json!(output))
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

#[cfg(any(feature = "mcp", feature = "grpc"))]
struct Identity(String);

#[cfg(any(feature = "mcp", feature = "grpc"))]
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

    async fn member_active(&self, tenant: &str, user: &str) -> Result<bool> {
        Ok(tenant == self.0 && user == "executor")
    }
    async fn session_active(&self, session: &str, tenant: &str, user: &str) -> Result<bool> {
        Ok(session == "executor-test" && self.member_active(tenant, user).await?)
    }
}

#[cfg(any(feature = "mcp", feature = "grpc"))]
#[async_trait::async_trait]
impl az_plugin_host::authorization::AuthorizationProvider for Identity {
    async fn can_publish(&self, _: &SessionContext) -> Result<bool> {
        Ok(false)
    }
}

#[tokio::test]
async fn external_executor_uses_public_plugin_contract_without_database_or_component_storage()
-> Result<()> {
    let root = tempfile::tempdir()?;
    let calls = Arc::new(AtomicUsize::new(0));
    let state = RuntimeState::initialize(
        config(root.path()),
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
                Default::default(),
                None
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

fn config(root: &std::path::Path) -> HostConfig {
    HostConfig {
        remote_capabilities: None,
        database_url: "deliberately invalid: no database".into(),
        public_origin: "http://127.0.0.1".into(),
        cache_root: root.join("cache"),
        component_storage: None,
        default_plugins: vec![],
        delivery: None,
        development: None,
    }
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
async fn explicitly_selected_script_runtime_rejects_zero_executors() -> Result<()> {
    let root = tempfile::tempdir()?;
    let result =
        RuntimeState::initialize(config(root.path()), vec![Arc::new(ScriptRuntimePlugin)]).await;
    let error = result.err().context("空脚本运行时被接受")?;
    assert!(format!("{error:#}").contains("未装配任何脚本执行器"));
    assert!(!root.path().join("cache").exists());
    Ok(())
}

#[cfg(any(feature = "rhai", feature = "typescript"))]
async fn language_executor(
    plugin: az_plugin_core::DynPlugin<RuntimeAssembly>,
    language: Language,
) -> Result<()> {
    let root = tempfile::tempdir()?;
    let state = RuntimeState::initialize(
        config(root.path()),
        vec![Arc::new(ScriptRuntimePlugin), plugin],
    )
    .await?;
    let runtime = state.service::<ScriptRuntime>()?;
    let mut package = package()?;
    package.enhancements[0].language = language;
    if language == Language::TypeScript {
        package.enhancements[0].source =
            "export function invoke(input: number): number { return input * 2; }".into();
    }
    let prepared = runtime.prepare(package.clone())?;
    assert_eq!(
        prepared
            .invocation(
                ScriptContext::default(),
                vec!["/v1/math/double".into()],
                Default::default(),
                None
            )
            .invoke("/v1/math/double", &json!(21))?,
        json!(42)
    );
    package.enhancements[0].language = match language {
        Language::Rhai => Language::TypeScript,
        Language::TypeScript => Language::Rhai,
    };
    let error = runtime
        .prepare(package)
        .err()
        .context("未选择的语言被接受")?;
    assert!(format!("{error:#}").contains("未装配支持该语言的脚本执行器"));
    state.shutdown().await?;
    Ok(())
}

#[cfg(feature = "rhai")]
#[tokio::test]
async fn rhai_can_run_without_the_typescript_plugin() -> Result<()> {
    language_executor(
        Arc::new(az_plugin_host::runtime::server::plugins::RhaiPlugin),
        Language::Rhai,
    )
    .await
}

#[cfg(feature = "typescript")]
#[tokio::test]
async fn typescript_can_run_without_the_rhai_plugin() -> Result<()> {
    language_executor(
        Arc::new(az_plugin_host::runtime::server::plugins::TypeScriptPlugin),
        Language::TypeScript,
    )
    .await
}

#[cfg(any(feature = "mcp", feature = "grpc"))]
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
        let state = RuntimeState::initialize(
            HostConfig {
                remote_capabilities: None,
                database_url: database.clone(),
                public_origin: base.clone(),
                cache_root: root.path().join("cache"),
                component_storage: None,
                default_plugins: vec![],
                delivery: None,
                development: None,
            },
            identity_support::with_identity(
                identity.clone(),
                vec![
                    #[cfg(feature = "grpc")]
                    Arc::new(GrpcPlugin),
                    #[cfg(feature = "mcp")]
                    Arc::new(McpPlugin),
                    Arc::new(EnhancementsPlugin),
                    Arc::new(EnhancementServicePlugin),
                    Arc::new(CapabilityServicePlugin),
                    Arc::new(ScriptRuntimePlugin),
                    Arc::new(ReplacementPlugin(calls.clone())),
                    Arc::new(DatabasePlugin),
                    Arc::new(PostgresPlugin),
                ],
            ),
        )
        .await?;
        let router = az_plugin_host::runtime::server::router(state.clone())?;
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
            #[cfg(feature = "grpc")]
            {
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
            }
            #[cfg(feature = "mcp")]
            {
                use rmcp::{
                    ServiceExt,
                    model::CallToolRequestParams,
                    transport::{
                        StreamableHttpClientTransport,
                        streamable_http_client::StreamableHttpClientTransportConfig,
                    },
                };
                let config = StreamableHttpClientTransportConfig::with_uri(format!(
                    "{base}/api/runtime/mcp"
                ))
                .auth_header("executor-test");
                let transport = StreamableHttpClientTransport::with_client(
                    reqwest_mcp::Client::builder().no_proxy().build()?,
                    config,
                );
                let client = ().serve(transport).await?;
                let output = async {
                    let tools = client.list_all_tools().await?;
                    assert_eq!(tools.len(), 1);
                    assert_eq!(tools[0].name, "v1.math.double");
                    let response = client
                        .call_tool(CallToolRequestParams::new("v1.math.double").with_arguments(
                            json!({"input":21}).as_object().context("参数无效")?.clone(),
                        ))
                        .await?;
                    assert_ne!(response.is_error, Some(true));
                    let content = response.structured_content.context("缺少 MCP 输出")?;
                    assert_eq!(content["output"], 42);
                    assert_eq!(content["revision"], revision);
                    Ok::<_, anyhow::Error>(())
                }
                .await;
                client.cancel().await?;
                output?;
            }
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
