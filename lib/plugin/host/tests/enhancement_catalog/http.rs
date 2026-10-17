use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use super::*;
use anyhow::{Context, Result};
use axum::http::HeaderMap;
use az_plugin_contract::grpc::{InvokeRequest, enhancements_client::EnhancementsClient};
use az_plugin_core::{DynPlugin, Plugin, PluginFuture};
use az_plugin_host::{
    authorization::AuthorizationProvider,
    configuration::HostConfig,
    identity::{IdentityProvider, SessionContext},
    runtime::server::plugins::*,
};
use rmcp::{
    ServiceExt,
    model::CallToolRequestParams,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::Value;

struct Identity(AtomicBool);
#[async_trait::async_trait]
impl IdentityProvider for Identity {
    async fn authenticate(&self, headers: &HeaderMap) -> Result<Option<SessionContext>> {
        let Some(actor) = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
        else {
            return Ok(None);
        };
        Ok(Some(SessionContext {
            session_id: actor.into(),
            user_id: actor.into(),
            account: actor.into(),
            display_name: actor.into(),
            tenant_id: if actor == "foreign" {
                "foreign"
            } else {
                "tenant"
            }
            .into(),
            tenant_label: "Test".into(),
            permissions: if self.0.load(Ordering::SeqCst) || actor == "denied" {
                vec![]
            } else if actor == "manager" || actor == "foreign" {
                vec!["plugin:manage".into()]
            } else {
                vec![format!("enhancement:{PATH}")]
            },
        }))
    }
    async fn member_active(&self, _: &str, _: &str) -> Result<bool> {
        Ok(true)
    }
    async fn session_active(&self, _: &str, _: &str, _: &str) -> Result<bool> {
        Ok(!self.0.load(Ordering::SeqCst))
    }
}
#[async_trait::async_trait]
impl AuthorizationProvider for Identity {
    async fn can_publish(&self, _: &SessionContext) -> Result<bool> {
        Ok(false)
    }
}

struct Storage(Arc<file::FileCatalog>);
impl Plugin<RuntimeAssembly> for Storage {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_builder(self.0.clone())
                .bind::<dyn EnhancementCatalog, file::FileCatalog>();
            Ok(())
        })
    }
}

struct DuplicateStorage(Arc<file::FileCatalog>);
impl Plugin<RuntimeAssembly> for DuplicateStorage {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_builder(self.0.clone())
                .bind::<dyn EnhancementCatalog, file::FileCatalog>()
                .bind::<dyn EnhancementCatalog, file::FileCatalog>();
            Ok(())
        })
    }
}

struct Host {
    host: RuntimeHost,
    plugins: Vec<DynPlugin<RuntimeAssembly>>,
    identity: Arc<Identity>,
    base: String,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
    client: reqwest::Client,
}
impl Drop for Host {
    fn drop(&mut self) {
        self.server.abort();
    }
}

fn selected(
    catalog: Arc<file::FileCatalog>,
    identity: Arc<Identity>,
) -> Vec<DynPlugin<RuntimeAssembly>> {
    identity_support::with_identity(
        identity,
        vec![
            Arc::new(Storage(catalog)),
            Arc::new(ScriptRuntimePlugin),
            Arc::new(RhaiPlugin),
            Arc::new(TypeScriptPlugin),
            Arc::new(EnhancementServicePlugin),
            Arc::new(EnhancementsPlugin),
            Arc::new(CapabilityServicePlugin),
            Arc::new(McpPlugin),
            Arc::new(GrpcPlugin),
        ],
    )
}

fn config(root: &Path, base: &str) -> HostConfig {
    HostConfig {
        database_url: "intentionally invalid database".into(),
        public_origin: base.into(),
        cache_root: root.join("cache"),
        component_storage: None,
        default_plugins: vec![],
        delivery: None,
        development: None,
        remote_capabilities: None,
    }
}

impl Host {
    async fn open(root: &Path, catalog: Arc<file::FileCatalog>) -> Result<Self> {
        let identity = Arc::new(Identity(AtomicBool::new(false)));
        let plugins = selected(catalog, identity.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let host = RuntimeHost::initialize_with_router(
            config(root, &base),
            plugins.clone(),
            |state, router| {
                #[cfg(feature = "postgres")]
                assert!(state.service::<sqlx::PgPool>().is_err());
                assert!(state.service::<az_plugin_runtime::Keyring>().is_err());
                Ok(router)
            },
        )
        .await?;
        let router = host.router();
        let server = tokio::spawn(async move { axum::serve(listener, router).await });
        Ok(Self {
            host,
            plugins,
            identity,
            base,
            server,
            client: reqwest::Client::new(),
        })
    }
    async fn propose(&self, package: &Package) -> Result<reqwest::Response> {
        Ok(self
            .client
            .post(format!("{}/api/runtime/enhancements/revisions", self.base))
            .bearer_auth("manager")
            .json(package)
            .send()
            .await?)
    }
    async fn change(
        &self,
        revision: Option<&str>,
        generation: i64,
        rollback: bool,
    ) -> Result<reqwest::Response> {
        let request = if rollback {
            self.client
                .post(format!("{}/api/runtime/enhancements/rollback", self.base))
        } else {
            self.client
                .put(format!("{}/api/runtime/enhancements/active", self.base))
        };
        Ok(request
            .bearer_auth("manager")
            .json(&json!({"revision":revision,"expected_generation":generation}))
            .send()
            .await?)
    }
    async fn compare(&self, revision: &str) -> Result<()> {
        let transport = StreamableHttpClientTransport::with_client(
            reqwest_mcp::Client::new(),
            StreamableHttpClientTransportConfig::with_uri(format!("{}/api/runtime/mcp", self.base))
                .auth_header("reader"),
        );
        let mut mcp = ().serve(transport).await?;
        assert_eq!(mcp.list_all_tools().await?.len(), 1);
        let output = mcp
            .call_tool(
                CallToolRequestParams::new("v1.math.double")
                    .with_arguments(serde_json::Map::from_iter([("input".into(), json!(7))])),
            )
            .await?;
        assert_ne!(output.is_error, Some(true));
        assert_eq!(
            output.structured_content.context("MCP 缺少输出")?,
            json!({"revision":revision,"output":14})
        );
        let mut grpc = EnhancementsClient::connect(self.base.clone()).await?;
        let mut request = tonic::Request::new(InvokeRequest {
            path: PATH.into(),
            input_json: serde_json::to_vec(&json!(7))?,
        });
        request
            .metadata_mut()
            .insert("authorization", "Bearer reader".parse()?);
        let response = grpc.invoke(request).await?.into_inner();
        assert_eq!(response.revision, revision);
        assert_eq!(
            serde_json::from_slice::<Value>(&response.output_json)?,
            json!(14)
        );
        mcp.close().await?;
        Ok(())
    }
    async fn close(&self) -> Result<()> {
        self.host.shutdown().await?;
        self.server.abort();
        Ok(())
    }
}

#[tokio::test]
async fn dual_language_http_mcp_grpc_survive_storage_replacement_without_database() -> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("enhancements.json");
    let catalog = file::FileCatalog::open(&path);
    let mut host = Host::open(root.path(), catalog.clone()).await?;
    let first = package(Language::Rhai);
    let first_revision = first.revision()?;
    host.propose(&first).await?.error_for_status()?;
    host.change(Some(&first_revision), 0, false)
        .await?
        .error_for_status()?;
    host.compare(&first_revision).await?;
    assert_eq!(
        host.change(Some(&first_revision), 0, false).await?.status(),
        409
    );
    let foreign = host
        .client
        .get(format!(
            "{}/api/runtime/enhancements/revisions/{first_revision}",
            host.base
        ))
        .bearer_auth("foreign")
        .send()
        .await?;
    assert!(!foreign.status().is_success());
    let denied = host
        .client
        .get(format!("{}/api/runtime/enhancements", host.base))
        .bearer_auth("denied")
        .send()
        .await?;
    assert_eq!(denied.status(), 403);
    let mut invalid = first.clone();
    invalid.enhancements[0].source = "fn invoke(input, context) { input * 3 }".into();
    assert!(!host.propose(&invalid).await?.status().is_success());
    assert_eq!(catalog.revisions("tenant").await?.len(), 1);

    let second = package(Language::TypeScript);
    let second_revision = second.revision()?;
    catalog.fail_commit.store(true, Ordering::SeqCst);
    assert!(!host.propose(&second).await?.status().is_success());
    assert!(catalog.package("tenant", &second_revision).await.is_err());
    catalog.fail_commit.store(false, Ordering::SeqCst);
    host.propose(&second).await?.error_for_status()?;
    catalog.fail_commit.store(true, Ordering::SeqCst);
    assert!(
        !host
            .change(Some(&second_revision), 1, false)
            .await?
            .status()
            .is_success()
    );
    catalog.fail_commit.store(false, Ordering::SeqCst);
    assert_eq!(catalog.head("tenant").await?.generation, 1);
    host.compare(&first_revision).await?;
    host.change(Some(&second_revision), 1, false)
        .await?
        .error_for_status()?;
    host.compare(&second_revision).await?;
    let observed = catalog.observe("tenant", &second_revision, 2).await?;
    assert_eq!(observed[0].succeeded, 2);

    // 提供者换成从磁盘重新打开的实例；宿主与执行链必须跟随 Dill 依赖重建。
    let reopened = file::FileCatalog::open(&path);
    for plugin in &mut host.plugins {
        if plugin.as_ref().type_id() == std::any::TypeId::of::<Storage>() {
            *plugin = Arc::new(Storage(reopened.clone()));
        }
    }
    host.host.reconfigure(host.plugins.clone()).await?;
    host.compare(&second_revision).await?;
    host.identity.0.store(true, Ordering::SeqCst);
    assert!(
        !host
            .change(Some(&first_revision), 2, true)
            .await?
            .status()
            .is_success()
    );
    host.identity.0.store(false, Ordering::SeqCst);
    host.change(Some(&first_revision), 2, true)
        .await?
        .error_for_status()?;
    host.compare(&first_revision).await?;
    host.close().await?;
    drop(host);
    let host = Host::open(root.path(), file::FileCatalog::open(&path)).await?;
    host.compare(&first_revision).await?;
    host.change(None, 3, false).await?.error_for_status()?;
    host.close().await?;
    drop(host);
    let host = Host::open(root.path(), file::FileCatalog::open(&path)).await?;
    assert_eq!(
        reopened.head("tenant").await?,
        Head {
            revision: None,
            generation: 4
        }
    );
    let mut changed = first.clone();
    changed.enhancements[0].input_schema = Some(json!({"type":"integer"}).try_into()?);
    host.propose(&changed).await?.error_for_status()?;
    assert!(
        !host
            .change(Some(&changed.revision()?), 4, false)
            .await?
            .status()
            .is_success()
    );
    host.change(Some(&second_revision), 4, true)
        .await?
        .error_for_status()?;
    host.compare(&second_revision).await?;
    assert!(!root.path().join("cache").exists());
    host.close().await
}

#[tokio::test]
async fn missing_or_duplicate_catalog_is_rejected_before_resource_creation() -> Result<()> {
    let root = tempfile::tempdir()?;
    let catalog = file::FileCatalog::open(&root.path().join("catalog.json"));
    let mut plugins = selected(catalog, Arc::new(Identity(AtomicBool::new(false))));
    plugins.retain(|plugin| plugin.as_ref().type_id() != std::any::TypeId::of::<Storage>());
    assert!(
        RuntimeHost::initialize(config(root.path(), "http://127.0.0.1"), plugins)
            .await
            .is_err()
    );
    let catalog = file::FileCatalog::open(&root.path().join("catalog.json"));
    let mut plugins = selected(catalog.clone(), Arc::new(Identity(AtomicBool::new(false))));
    plugins.retain(|plugin| plugin.as_ref().type_id() != std::any::TypeId::of::<Storage>());
    plugins.push(Arc::new(DuplicateStorage(catalog)));
    assert!(
        RuntimeHost::initialize(config(root.path(), "http://127.0.0.1"), plugins)
            .await
            .is_err()
    );
    assert!(!root.path().join("catalog.json").exists());
    Ok(())
}
