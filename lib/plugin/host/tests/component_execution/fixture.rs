use anyhow::{Context, Result, ensure};
use az_plugin_bundle::{Bundle, VerifiedBundle};
use az_plugin_contract::RequestContext;
use az_plugin_core::{DynPlugin, Plugin, PluginFuture};
use az_plugin_host::{
    configuration::{ComponentStorage, HostConfig},
    identity::{IdentityProvider, SessionContext},
    runtime::server::{RuntimeState, plugins::*},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    },
};
use tokio::sync::{Mutex, Notify};
use uuid::Uuid;

#[derive(Default)]
pub struct Probe {
    pub context: Mutex<Option<ExecutionContext>>,
    pub active: Mutex<HashMap<(Uuid, String), Arc<VerifiedBundle>>>,
    pub events: Mutex<Vec<&'static str>>,
    pub pool: Mutex<Option<Arc<PgPool>>>,
    pub fail_activate: AtomicBool,
    pub fail_deactivate: AtomicBool,
    pub initialization: AtomicU8,
    pub panic_stop: AtomicBool,
    pub accept_all: AtomicBool,
    pub stopped: AtomicUsize,
    pub entered: Notify,
    pub resume: Notify,
}

pub struct Replacement<const ID: u8>(pub Arc<Probe>);

#[async_trait::async_trait]
impl<const ID: u8> ComponentExecutor for Replacement<ID> {
    fn supports(&self, bundle: &VerifiedBundle) -> bool {
        self.0.accept_all.load(Ordering::SeqCst)
            || bundle.manifest().plugin.runtime.artifact == format!("server-{ID}")
    }
    async fn initialize(&self, context: ExecutionContext) -> Result<()> {
        *self.0.context.lock().await = Some(context.clone());
        self.0.events.lock().await.push("initialize");
        *self.0.pool.lock().await = Some(context.services()?.get::<dill::OneOf<PgPool>>()?);
        self.0.entered.notify_one();
        match self.0.initialization.load(Ordering::SeqCst) {
            1 => anyhow::bail!("测试初始化失败"),
            2 => panic!("测试初始化 panic"),
            3 => self.0.resume.notified().await,
            _ => {}
        }
        Ok(())
    }
    async fn validate(&self, _: Uuid, bundle: &Bundle) -> Result<ComponentDescription> {
        self.0.events.lock().await.push("validate");
        // 刻意与包内 process 标记不同，证明展示字段不能决定分发实现。
        ComponentDescription {
            process: false,
            label: "Replacement".into(),
            pages: vec![ComponentPage {
                id: "home".into(),
                label: "Home".into(),
                entry: "index.html".into(),
                scene: None,
                menu_path: vec![],
                permission: None,
                surface: "workspace".into(),
            }],
        }
        .with_settings(&bundle.verify()?)
    }
    async fn activate(&self, source: Uuid, tenant: &str, bundle: &Bundle) -> Result<()> {
        self.0.events.lock().await.push("activate");
        self.0
            .active
            .lock()
            .await
            .insert((source, tenant.into()), Arc::new(bundle.verify()?));
        ensure!(
            !self.0.fail_activate.load(Ordering::SeqCst),
            "测试候选激活失败"
        );
        Ok(())
    }
    async fn restore(&self, source: Uuid, tenant: &str, bundle: &Bundle) -> Result<()> {
        self.0.events.lock().await.push("restore");
        self.0
            .active
            .lock()
            .await
            .insert((source, tenant.into()), Arc::new(bundle.verify()?));
        Ok(())
    }
    async fn bundle(&self, source: Uuid, tenant: &str) -> Result<Option<Arc<VerifiedBundle>>> {
        Ok(self
            .0
            .active
            .lock()
            .await
            .get(&(source, tenant.into()))
            .cloned())
    }
    async fn handle(
        &self,
        source: Uuid,
        tenant: &str,
        digest: &str,
        request: ComponentRequest,
        context: RequestContext,
    ) -> Result<ComponentResponse> {
        let bundle = self
            .bundle(source, tenant)
            .await?
            .context("测试实例未激活")?;
        ensure!(
            bundle.digest() == digest && context.tenant_id.as_deref() == Some(tenant),
            "版本或租户错误"
        );
        self.0.events.lock().await.push("handle");
        Ok(ComponentResponse {
            status: 200,
            headers: vec![],
            body: serde_json::to_vec(
                &json!({"executor":ID,"tenant":tenant,"revision":digest,"path":request.path}),
            )?,
        })
    }
    async fn deactivate(&self, source: Uuid, tenant: &str) -> Result<()> {
        self.0.events.lock().await.push("deactivate");
        self.0.active.lock().await.remove(&(source, tenant.into()));
        ensure!(
            !self.0.fail_deactivate.load(Ordering::SeqCst),
            "测试清理候选失败"
        );
        Ok(())
    }
    async fn shutdown(&self) -> Result<()> {
        self.0.context.lock().await.take();
        self.0.events.lock().await.push("shutdown");
        self.0.active.lock().await.clear();
        self.0.stopped.fetch_add(1, Ordering::SeqCst);
        assert!(!self.0.panic_stop.load(Ordering::SeqCst), "测试关闭 panic");
        Ok(())
    }
}

pub struct Replacements(pub [Arc<Probe>; 2]);

impl Plugin<RuntimeAssembly> for Replacements {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_value(Replacement::<1>(self.0[0].clone()))
                .bind::<dyn ComponentExecutor, Replacement<1>>();
            assembly
                .services
                .add_value(Replacement::<2>(self.0[1].clone()))
                .bind::<dyn ComponentExecutor, Replacement<2>>();
            Ok(())
        })
    }
}

pub struct Identity(pub String);
#[async_trait::async_trait]
impl IdentityProvider for Identity {
    async fn authenticate(
        &self,
        headers: &axum::http::HeaderMap,
    ) -> Result<Option<SessionContext>> {
        if headers.get("cookie").and_then(|v| v.to_str().ok()) != Some("session=component-test") {
            return Ok(None);
        }
        Ok(Some(SessionContext {
            session_id: "component-test".into(),
            user_id: "reader".into(),
            account: "reader".into(),
            display_name: "Reader".into(),
            tenant_id: self.0.clone(),
            tenant_label: "Test".into(),
            permissions: vec!["plugin:manage".into()],
        }))
    }
    async fn can_publish(&self, _: &SessionContext) -> Result<bool> {
        Ok(true)
    }
    async fn member_active(&self, tenant: &str, user: &str) -> Result<bool> {
        Ok(tenant == self.0 && user == "reader")
    }
    async fn session_active(&self, session: &str, tenant: &str, user: &str) -> Result<bool> {
        Ok(session == "component-test" && self.member_active(tenant, user).await?)
    }
}

pub fn config(root: &Path, database: &str, origin: &str) -> HostConfig {
    HostConfig {
        remote_capabilities: None,
        database_url: database.into(),
        public_origin: origin.into(),
        cache_root: root.join("legacy"),
        component_storage: Some(ComponentStorage {
            database_url: database.into(),
            root: root.join("components"),
        }),
        default_plugins: vec![],
        delivery: None,
        development: None,
    }
}

pub fn plugins(probes: [Arc<Probe>; 2], surfaces: bool) -> Vec<DynPlugin<RuntimeAssembly>> {
    let mut plugins: Vec<DynPlugin<RuntimeAssembly>> = vec![
        Arc::new(ComponentsPlugin),
        Arc::new(ComponentResourcesPlugin),
        Arc::new(PostgresComponentPlugin),
        Arc::new(FileObjectStoragePlugin),
        Arc::new(ComponentExecutionPlugin),
        Arc::new(Replacements(probes)),
        Arc::new(StorePlugin),
        Arc::new(DatabasePlugin),
        Arc::new(PostgresPlugin),
        Arc::new(KeyringPlugin),
        Arc::new(FileKeyringPlugin),
        Arc::new(EnhancementServicePlugin),
        Arc::new(ScriptRuntimePlugin),
        Arc::new(RhaiPlugin),
    ];
    if surfaces {
        // 管理与页面路由只依赖公共执行契约，替代执行器不引入内置引擎。
        plugins.extend([
            Arc::new(ManagementPlugin) as DynPlugin<RuntimeAssembly>,
            Arc::new(FrontendPlugin),
            Arc::new(RepositoryPlugin),
        ]);
    }
    plugins
}

pub async fn database() -> Result<String> {
    let database = std::env::var("AIO_COMPONENT_TEST_DATABASE_URL")?;
    let url = reqwest::Url::parse(&database)?;
    ensure!(
        matches!(url.host_str(), Some("127.0.0.1" | "localhost"))
            && url.path().ends_with("component_market_test"),
        "只接受本机独立组件测试库"
    );
    Ok(database)
}

pub fn package(root: &Path, git: &str, executor: u8, version: &str) -> Result<Bundle> {
    let root = root.join(version);
    std::fs::create_dir_all(root.join("web"))?;
    std::fs::write(
        root.join("web/index.html"),
        "<html><head></head><body>Executor</body></html>",
    )?;
    let mut binary = vec![0; 64];
    binary[..6].copy_from_slice(b"\x7fELF\x02\x01");
    binary[18] = 62;
    std::fs::write(root.join(format!("server-{executor}")), binary)?;
    std::fs::write(
        root.join("aio-plugin.toml"),
        format!(
            "schema_version=2\n[plugin.marketplace]\ntitle='Executor'\nsummary='Replacement verification'\nlicense='MIT'\n[plugin.runtime]\nartifact='server-{executor}'\nhost_version='>=2026.9.21'\n[plugin.runtime.process]\nimage='sha256:{}'\n[plugin.frontend]\npath='web'\n",
            "a".repeat(64)
        ),
    )?;
    Bundle::from_directory(
        &root,
        "aio-plugin.toml",
        git.into(),
        "a".repeat(40),
        version.into(),
    )
}

pub struct Host {
    pub state: RuntimeState,
    pub base: String,
    pub client: reqwest::Client,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
}
impl Host {
    pub async fn start(
        root: &Path,
        database: &str,
        tenant: &str,
        probes: [Arc<Probe>; 2],
    ) -> Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let state = RuntimeState::initialize_with_plugins(
            config(root, database, &base),
            Arc::new(Identity(tenant.into())),
            plugins(probes, true),
        )
        .await?;
        let router = az_plugin_host::runtime::server::router(state.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await });
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("cookie", "session=component-test".parse()?);
        let client = reqwest::Client::builder()
            .no_proxy()
            .default_headers(headers)
            .build()?;
        Ok(Self {
            state,
            base,
            server,
            client,
        })
    }
    pub async fn stop(self) -> Result<()> {
        let result = self.state.shutdown().await;
        self.server.abort();
        let _ = self.server.await;
        result
    }
    pub async fn publish(&self, bundle: &Bundle) -> Result<reqwest::Response> {
        Ok(self
            .client
            .post(format!("{}/api/runtime/components/publish", self.base))
            .header("content-type", "application/vnd.aio.component+gzip")
            .body(bundle.encode()?)
            .send()
            .await?)
    }
    pub async fn install(&self, bundle: &Bundle) -> Result<reqwest::Response> {
        self.post(
            "/api/runtime/plugins/install",
            json!({"git":bundle.git,"rev":bundle.digest}),
        )
        .await
    }
    pub async fn post(&self, path: &str, body: Value) -> Result<reqwest::Response> {
        Ok(self
            .client
            .post(format!("{}{path}", self.base))
            .json(&body)
            .send()
            .await?)
    }
    pub async fn mount(&self, source: Uuid) -> Result<String> {
        let response = self
            .post(
                "/api/runtime/frontend/mount",
                json!({"page_id":format!("component:{source}:home")}),
            )
            .await?;
        let value = success(response).await?;
        Ok(value["data"]["token"]
            .as_str()
            .context("缺少挂载票据")?
            .into())
    }
    pub async fn invoke(&self, token: &str) -> Result<reqwest::Response> {
        self.post(
            &format!("/api/runtime/components/{token}/request"),
            json!({"method":"GET","path":"/test","body":[],"headers":[]}),
        )
        .await
    }
    pub async fn assert_call(&self, source: Uuid, executor: u8, bundle: &Bundle) -> Result<String> {
        let token = self.mount(source).await?;
        let response = success(self.invoke(&token).await?).await?;
        let bytes: Vec<u8> = serde_json::from_value(response["data"]["body"].clone())?;
        let output: Value = serde_json::from_slice(&bytes)?;
        assert_eq!(output["executor"], executor);
        assert_eq!(output["revision"], bundle.digest);
        Ok(token)
    }
}

pub async fn success(response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    let text = response.text().await?;
    ensure!(status.is_success(), "HTTP {status}: {text}");
    Ok(serde_json::from_str(&text)?)
}

pub async fn committed_failure(pool: &PgPool, bundle: &Bundle) -> Result<()> {
    // 延迟约束在 COMMIT 才失败，验证候选已上线但安装事务无法提交时的补偿。
    let sql = format!(
        "CREATE FUNCTION reject_executor_commit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF (CASE WHEN TG_OP = 'DELETE' THEN OLD.digest ELSE NEW.digest END) = '{}' THEN RAISE EXCEPTION 'injected commit failure'; END IF; RETURN NULL; END $$; CREATE CONSTRAINT TRIGGER reject_executor_commit AFTER INSERT OR UPDATE OR DELETE ON component_installations DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION reject_executor_commit();",
        bundle.digest
    );
    sqlx::raw_sql(&sql).execute(pool).await?;
    Ok(())
}

pub async fn clear_failure(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql("DROP TRIGGER reject_executor_commit ON component_installations; DROP FUNCTION reject_executor_commit();").execute(pool).await?;
    Ok(())
}
