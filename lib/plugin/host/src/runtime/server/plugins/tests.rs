use std::any::Any;

use axum::{extract::State, routing::get};
use az_plugin_core::{Plugin, PluginFuture};

use super::*;
use crate::generated::worker::WorkerService;

trait Probe: Any + Send + Sync {
    fn value(&self) -> &'static str;
}

pub(super) struct NoIdentity;

#[async_trait::async_trait]
impl crate::identity::IdentityProvider for NoIdentity {
    async fn authenticate(
        &self,
        _: &axum::http::HeaderMap,
    ) -> Result<Option<crate::identity::SessionContext>> {
        Ok(None)
    }

    async fn member_active(&self, _: &str, _: &str) -> Result<bool> {
        Ok(false)
    }
    async fn session_active(&self, _: &str, _: &str, _: &str) -> Result<bool> {
        Ok(false)
    }
}

#[async_trait::async_trait]
impl crate::authorization::AuthorizationProvider for NoIdentity {
    async fn can_publish(&self, _: &crate::identity::SessionContext) -> Result<bool> {
        Ok(false)
    }
}

struct Replacement;
impl Probe for Replacement {
    fn value(&self) -> &'static str {
        "replacement"
    }
}

struct ProbePlugin;

#[derive(Default)]
struct Trace(std::sync::Mutex<Vec<&'static str>>);

impl Plugin<RuntimeAssembly> for ProbePlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_value(Replacement)
                .bind::<dyn Probe, Replacement>();
            assembly.require_service::<dyn Probe>();
            assembly.services.add_value(Trace::default());
            assembly.on_start::<Self>(|state, _tasks| {
                Box::pin(async move {
                    let trace = state.service::<Trace>()?;
                    trace
                        .0
                        .lock()
                        .map_err(|_| anyhow::anyhow!("测试状态锁损坏"))?
                        .push("start");
                    Ok(())
                })
            });
            assembly.on_ready::<Self>(|state, _tasks| {
                Box::pin(async move {
                    let trace = state.service::<Trace>()?;
                    let mut events = trace
                        .0
                        .lock()
                        .map_err(|_| anyhow::anyhow!("测试状态锁损坏"))?;
                    anyhow::ensure!(*events == ["start"], "插件必须恢复完成后才能就绪");
                    events.push("ready");
                    Ok(())
                })
            });
            assembly.add_routes(|_| Router::new().route("/api/runtime/probe", get(probe)));
            Ok(())
        })
    }
}

async fn probe(
    State(state): State<RuntimeState>,
) -> Result<String, super::super::http_error::RuntimeError> {
    Ok(state.service::<dyn Probe>()?.value().into())
}

pub(super) async fn extensions(
    plugins: Vec<DynPlugin<RuntimeAssembly>>,
) -> Result<Arc<RuntimeExtensions>> {
    assemble(Arc::new(config(std::path::Path::new("/unused"))), plugins).await
}

pub(super) fn config(root: &std::path::Path) -> crate::configuration::HostConfig {
    crate::configuration::HostConfig {
        remote_capabilities: None,
        database_url: "deliberately invalid: no database".into(),
        cache_root: root.join("cache"),
        public_origin: "http://127.0.0.1".into(),
        component_storage: None,
        default_plugins: Vec::new(),
        delivery: None,
        development: None,
    }
}

#[tokio::test]
async fn empty_profile_has_no_implicit_services_or_routes() -> Result<()> {
    let extensions = extensions(Vec::new()).await?;
    assert!(extensions.routes.is_empty());
    assert!(extensions.services.get_one::<dyn WorkerService>().is_err());
    Ok(())
}

#[tokio::test]
async fn remote_configuration_requires_its_feature_and_plugin() -> Result<()> {
    let mut config = config(std::path::Path::new("/unused"));
    config.remote_capabilities = Some("/unused/remote.json".into());
    let error = assemble(Arc::new(config), vec![])
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("未装配远端能力插件仍接受配置"))?;
    #[cfg(feature = "remote")]
    assert!(error.to_string().contains("RemoteCapabilitiesPlugin"));
    #[cfg(not(feature = "remote"))]
    assert!(error.to_string().contains("remote feature"));
    Ok(())
}

#[tokio::test]
async fn default_profile_declares_services_and_rejects_missing_requirements() -> Result<()> {
    let extensions = extensions(crate::runtime::server::admin_test_support::with_identity(
        Arc::new(NoIdentity),
        default_plugins()?,
    ))
    .await?;
    // 声明阶段只校验绑定，不会提前创建尚未初始化的数据库服务。
    assert_eq!(
        extensions
            .services
            .builders_for::<dyn WorkerService>()
            .count(),
        1
    );
    assert!(extensions.services.get_one::<sqlx::PgPool>().is_err());
    assert!(
        self::extensions(vec![Arc::new(ClipboardPlugin)])
            .await
            .is_err()
    );
    assert!(
        self::extensions(vec![Arc::new(ProbePlugin), Arc::new(ProbePlugin)])
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn application_can_install_its_own_service_and_route_provider() -> Result<()> {
    let extensions = extensions(vec![Arc::new(ProbePlugin)]).await?;
    assert_eq!(
        extensions.services.get_one::<dyn Probe>()?.value(),
        "replacement"
    );
    assert!(extensions.services.get_one::<dyn WorkerService>().is_err());
    Ok(())
}

#[tokio::test]
async fn selected_plugins_control_real_http_routes() -> Result<()> {
    let root = tempfile::tempdir()?;
    let state = RuntimeState::initialize(config(root.path()), vec![Arc::new(ProbePlugin)]).await?;
    assert!(
        state
            .service::<dyn crate::identity::IdentityProvider>()
            .is_err()
    );
    assert!(
        state
            .service::<dyn crate::authorization::AuthorizationProvider>()
            .is_err()
    );
    assert!(state.service::<sqlx::PgPool>().is_err());
    assert!(state.service::<az_plugin_runtime::Keyring>().is_err());
    assert!(state.service::<dyn super::WasmExecutor>().is_err());
    assert!(!root.path().join("cache").exists());
    let trace = state.service::<Trace>()?;
    assert_eq!(
        *trace
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("测试状态锁损坏"))?,
        ["start", "ready"]
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let router = super::super::router(state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let client = reqwest::Client::builder().no_proxy().build()?;
    let outcome = async {
        let response = client
            .get(format!("{origin}/api/runtime/probe"))
            .send()
            .await?
            .error_for_status()?;
        assert_eq!(response.text().await?, "replacement");
        for endpoint in [
            "/api/runtime/catalog",
            "/api/runtime/clipboard/head",
            "/api/runtime/enhancements",
            "/api/runtime/workers",
        ] {
            assert_eq!(
                client
                    .get(format!("{origin}{endpoint}"))
                    .send()
                    .await?
                    .status(),
                404
            );
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    state.shutdown().await?;
    server.abort();
    outcome
}
