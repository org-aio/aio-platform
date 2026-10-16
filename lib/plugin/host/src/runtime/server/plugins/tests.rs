use std::any::Any;

use axum::{extract::State, routing::get};
use az_plugin_core::{Plugin, PluginFuture};

use super::*;
use crate::generated::worker::WorkerService;

trait Probe: Any + Send + Sync {
    fn value(&self) -> &'static str;
}

struct NoIdentity;

#[async_trait::async_trait]
impl crate::identity::IdentityProvider for NoIdentity {
    async fn authenticate(
        &self,
        _: &axum::http::HeaderMap,
    ) -> Result<Option<crate::identity::SessionContext>> {
        Ok(None)
    }
    async fn can_publish(&self, _: &crate::identity::SessionContext) -> Result<bool> {
        Ok(false)
    }
    async fn member_active(&self, _: &str, _: &str) -> Result<bool> {
        Ok(false)
    }
    async fn session_active(&self, _: &str, _: &str, _: &str) -> Result<bool> {
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
            assembly.on_start(|state| {
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
            assembly.on_ready(|state| {
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

async fn extensions(plugins: Vec<DynPlugin<RuntimeAssembly>>) -> Result<Arc<RuntimeExtensions>> {
    let root = tempfile::tempdir()?;
    let keyring = super::super::components::load_keyring(&root.path().join("keyring.json"))?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgresql://test@127.0.0.1/kernel_test")?;
    assemble(pool, keyring, plugins).await
}

#[tokio::test]
async fn empty_profile_has_no_implicit_services_or_routes() -> Result<()> {
    let extensions = extensions(Vec::new()).await?;
    assert!(extensions.routes.is_empty());
    assert!(extensions.services.get_one::<dyn WorkerService>().is_err());
    Ok(())
}

#[tokio::test]
async fn default_profile_resolves_shared_services_and_rejects_missing_requirements() -> Result<()> {
    let extensions = extensions(default_plugins()?).await?;
    let first = extensions.services.get_one::<dyn WorkerService>()?;
    let second = extensions.services.get_one::<dyn WorkerService>()?;
    assert!(Arc::ptr_eq(&first, &second));
    extensions
        .services
        .get_one::<dyn crate::generated::clipboard::ClipboardService>()?;
    extensions
        .services
        .get_one::<dyn crate::generated::personal_config::PersonalConfigService>()?;
    extensions
        .services
        .get_one::<dyn crate::generated::worker_webview::WorkerWebviewService>()?;
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
#[ignore = "需要独立本机 PostgreSQL"]
async fn selected_plugins_control_real_http_routes() -> Result<()> {
    let database = std::env::var("AIO_ENHANCEMENT_TEST_DATABASE_URL")?;
    anyhow::ensure!(
        database.contains("127.0.0.1") && database.contains("_test"),
        "只接受本机独立测试库"
    );
    let root = tempfile::tempdir()?;
    let state = RuntimeState::initialize_with_plugins(
        crate::configuration::HostConfig {
            database_url: database,
            cache_root: root.path().join("cache"),
            public_origin: "http://127.0.0.1".into(),
            component_storage: None,
            default_plugins: Vec::new(),
            delivery: None,
            development: None,
        },
        Arc::new(NoIdentity),
        vec![Arc::new(ProbePlugin)],
    )
    .await?;
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
    let router = super::super::router(state);
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
    server.abort();
    outcome
}
