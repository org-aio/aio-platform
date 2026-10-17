#[cfg(feature = "sidecar")]
use std::str::FromStr;
use std::{
    any::TypeId,
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    },
};

use anyhow::{Context, Result, ensure};
use az_plugin_bundle::Bundle;
use az_plugin_core::{DynPlugin, Plugin, PluginFuture};
use az_plugin_host::{
    configuration::ComponentStorage,
    runtime::server::{RuntimeState, plugins::*},
};
use az_plugin_runtime::{
    ComponentDatabase, DatabaseProvisioner, Keyring, ObjectKey, ObjectStorage, ScopedDatabase,
};
use sqlx::PgPool;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::fixture;

struct PreparedDatabase {
    pool: PgPool,
    bindings: Mutex<HashMap<(String, String), ScopedDatabase>>,
    calls: AtomicUsize,
    stops: AtomicUsize,
    wrong_scope: AtomicBool,
}

#[async_trait::async_trait]
impl ComponentDatabase for PreparedDatabase {
    async fn install(
        &self,
        source: &str,
        tenant: &str,
        migrations: &[(String, String)],
        _: &Keyring,
    ) -> Result<ScopedDatabase> {
        ensure!(!migrations.is_empty(), "测试迁移未交给替代实现");
        self.calls.fetch_add(1, Ordering::SeqCst);
        let bindings = self.bindings.lock().await;
        if self.wrong_scope.load(Ordering::SeqCst) {
            return bindings.values().next().cloned().context("缺少测试绑定");
        }
        bindings
            .get(&(source.into(), tenant.into()))
            .cloned()
            .context("未预先分配租户数据库")
    }
    fn releases(&self) -> Arc<dyn az_plugin_runtime::ComponentReleases> {
        Arc::new(az_plugin_runtime::PostgresReleases::new(self.pool.clone()))
    }
    async fn shutdown(&self) -> Result<()> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        self.bindings.lock().await.clear();
        self.pool.close().await;
        Ok(())
    }
}

#[derive(Default)]
struct RemoteObjects {
    values: Mutex<HashMap<(String, String), Vec<u8>>>,
    stops: AtomicUsize,
    panic_stop: AtomicBool,
}

#[async_trait::async_trait]
impl ObjectStorage for RemoteObjects {
    async fn read(&self, key: &ObjectKey) -> Result<Vec<u8>> {
        self.values
            .lock()
            .await
            .get(&(key.namespace().into(), key.digest().into()))
            .cloned()
            .context("测试远端对象不存在")
    }
    async fn write(&self, key: &ObjectKey, bytes: &[u8]) -> Result<()> {
        self.values
            .lock()
            .await
            .insert((key.namespace().into(), key.digest().into()), bytes.into());
        Ok(())
    }
    async fn remove(&self, key: &ObjectKey) -> Result<()> {
        self.values
            .lock()
            .await
            .remove(&(key.namespace().into(), key.digest().into()));
        Ok(())
    }
    async fn shutdown(&self) -> Result<()> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        assert!(
            !self.panic_stop.load(Ordering::SeqCst),
            "测试对象后端关闭 panic"
        );
        Ok(())
    }
}

#[derive(Clone)]
struct Providers {
    database: Arc<PreparedDatabase>,
    objects: Arc<RemoteObjects>,
    storage_failure: Arc<AtomicU8>,
}
impl Providers {
    fn new(pool: PgPool) -> Self {
        Self {
            database: Arc::new(PreparedDatabase {
                pool,
                bindings: Default::default(),
                calls: Default::default(),
                stops: Default::default(),
                wrong_scope: Default::default(),
            }),
            objects: Arc::default(),
            storage_failure: Arc::default(),
        }
    }
}
#[async_trait::async_trait]
impl ComponentDatabaseProvider for Providers {
    async fn open(&self, config: &ComponentStorage) -> Result<Arc<dyn ComponentDatabase>> {
        assert_eq!(config.database_url, "unused component database");
        Ok(self.database.clone())
    }
}
#[async_trait::async_trait]
impl ObjectStorageProvider for Providers {
    async fn open(&self, _: &ComponentStorage) -> Result<Arc<dyn ObjectStorage>> {
        match self.storage_failure.load(Ordering::SeqCst) {
            1 => anyhow::bail!("测试对象后端打开失败"),
            2 => panic!("测试对象后端打开 panic"),
            _ => Ok(self.objects.clone()),
        }
    }
}
impl Plugin<RuntimeAssembly> for Providers {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_value(self.clone())
                .bind::<dyn ComponentDatabaseProvider, Self>()
                .bind::<dyn ObjectStorageProvider, Self>();
            Ok(())
        })
    }
}

fn plugins(
    providers: Providers,
    probes: [Arc<fixture::Probe>; 2],
) -> Vec<DynPlugin<RuntimeAssembly>> {
    let mut plugins = fixture::plugins(probes, false);
    plugins.retain(|plugin| {
        ![
            TypeId::of::<PostgresComponentPlugin>(),
            TypeId::of::<FileObjectStoragePlugin>(),
        ]
        .contains(&plugin.as_ref().type_id())
    });
    plugins.push(Arc::new(providers));
    plugins
}

fn config(root: &std::path::Path, database: &str) -> az_plugin_host::configuration::HostConfig {
    let mut config = fixture::config(root, database, "http://127.0.0.1");
    if let Some(storage) = &mut config.component_storage {
        storage.database_url = "unused component database".into();
    }
    config
}

fn package(root: &std::path::Path, git: &str, storage: bool) -> Result<Bundle> {
    fixture::package(root, git, 1, "1.0.0")?;
    let root = root.join("1.0.0");
    let manifest = root.join("aio-plugin.toml");
    let mut text = std::fs::read_to_string(&manifest)?;
    text.push_str(&format!("[plugin.capabilities]\ndatabase=true\nstorage={storage}\n[plugin.database]\nmigrations='migrations'\n"));
    std::fs::write(manifest, text)?;
    std::fs::create_dir_all(root.join("migrations"))?;
    std::fs::write(
        root.join("migrations/0001.sql"),
        "CREATE TABLE records(id BIGINT PRIMARY KEY, value TEXT)",
    )?;
    Bundle::from_directory(
        &root,
        "aio-plugin.toml",
        git.into(),
        "a".repeat(40),
        "1.0.0".into(),
    )
}

async fn binding(
    providers: &Providers,
    provisioner: &DatabaseProvisioner,
    source: Uuid,
    tenant: &str,
) -> Result<()> {
    // 测试准备阶段分配数据库；运行中的替代实现只领取已分配连接，不使用默认 provisioner。
    let database = provisioner
        .create(
            &source.to_string(),
            tenant,
            "CREATE TABLE records(id BIGINT PRIMARY KEY, value TEXT)",
        )
        .await?;
    providers
        .database
        .bindings
        .lock()
        .await
        .insert((source.to_string(), tenant.into()), database);
    Ok(())
}

#[tokio::test]
async fn missing_and_ambiguous_resource_providers_fail_before_opening_databases() -> Result<()> {
    let root = tempfile::tempdir()?;
    for duplicate in [false, true] {
        let providers = Providers::new(PgPool::connect_lazy(
            "postgresql://unused@127.0.0.1/resource_test",
        )?);
        let mut selected = fixture::plugins(Default::default(), false);
        if duplicate {
            selected.push(Arc::new(providers.clone()));
        } else {
            selected.retain(|plugin| {
                plugin.as_ref().type_id() != TypeId::of::<FileObjectStoragePlugin>()
            });
        }
        let result = RuntimeState::initialize(
            config(root.path(), "invalid database"),
            super::identity_support::with_identity(
                Arc::new(fixture::Identity("test".into())),
                selected,
            ),
        )
        .await;
        let error = result.err().context("资源装配应失败")?;
        assert!(format!("{error:#}").contains("唯一的服务绑定"), "{error:#}");
        assert_eq!(providers.database.stops.load(Ordering::SeqCst), 0);
        assert!(!root.path().join("components").exists());
        providers.database.pool.close().await;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn replacement_resources_share_scope_with_external_executors() -> Result<()> {
    let url = fixture::database().await?;
    let root = tempfile::tempdir()?;
    let source = Uuid::new_v4();
    let tenant = Uuid::new_v4().to_string();
    let providers = Providers::new(PgPool::connect(&url).await?);
    let provisioner = DatabaseProvisioner::connect(&url).await?;
    binding(&providers, &provisioner, source, &tenant).await?;
    binding(&providers, &provisioner, source, "foreign").await?;
    provisioner.shutdown().await?;
    let probes: [Arc<fixture::Probe>; 2] = Default::default();
    let state = RuntimeState::initialize(
        config(root.path(), &url),
        super::identity_support::with_identity(
            Arc::new(fixture::Identity(tenant.clone())),
            plugins(providers.clone(), probes.clone()),
        ),
    )
    .await?;
    let bundle = package(root.path(), "https://github.com/example/resource.git", true)?.verify()?;
    let first = probes[0]
        .context
        .lock()
        .await
        .clone()
        .context("执行器未收到上下文")?;
    let second = probes[1]
        .context
        .lock()
        .await
        .clone()
        .context("执行器未收到上下文")?;
    let resources = first.resources(source, &tenant, &bundle).await?;
    let store = resources.storage.as_ref().context("没有替代对象句柄")?;
    store.write("shared", b"from-first").await?;
    let resources2 = second.resources(source, &tenant, &bundle).await?;
    assert_eq!(
        resources2
            .storage
            .as_ref()
            .context("没有第二个句柄")?
            .read("shared")
            .await?,
        b"from-first"
    );
    let foreign = second.resources(source, "foreign", &bundle).await?;
    assert!(
        foreign
            .storage
            .as_ref()
            .context("没有隔离句柄")?
            .read("shared")
            .await
            .is_err()
    );
    let mut transaction = resources
        .database
        .as_ref()
        .context("没有数据库句柄")?
        .begin()
        .await?;
    transaction
        .execute("INSERT INTO records VALUES (1,'retained')", vec![])
        .await?;
    transaction.commit().await?;
    let mut transaction = resources2
        .database
        .as_ref()
        .context("没有共享数据库句柄")?
        .begin()
        .await?;
    assert_eq!(
        transaction
            .query("SELECT value FROM records WHERE id=1", vec![])
            .await?
            .values,
        vec![vec![az_plugin_runtime::DatabaseValue::Text(
            "retained".into()
        )]]
    );
    transaction.commit().await?;
    let calls = providers.database.calls.load(Ordering::SeqCst);
    let no_grants = fixture::package(
        &root.path().join("ungranted"),
        "https://github.com/example/ungranted.git",
        1,
        "1.0.0",
    )?
    .verify()?;
    let ungranted = first.resources(source, &tenant, &no_grants).await?;
    assert!(ungranted.database.is_none() && ungranted.storage.is_none());
    assert_eq!(providers.database.calls.load(Ordering::SeqCst), calls);
    providers.database.wrong_scope.store(true, Ordering::SeqCst);
    assert!(
        first
            .resources(source, "wrong-tenant", &bundle)
            .await
            .is_err()
    );
    assert!(!root.path().join("components/objects").exists());
    state.shutdown().await?;
    state.shutdown().await?;
    assert_eq!(providers.database.stops.load(Ordering::SeqCst), 1);
    assert_eq!(providers.objects.stops.load(Ordering::SeqCst), 1);
    assert!(providers.database.pool.is_closed());
    Ok(())
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn partial_resource_and_executor_failures_close_every_acquired_backend() -> Result<()> {
    let url = fixture::database().await?;
    for failure in 0..4 {
        let root = tempfile::tempdir()?;
        let providers = Providers::new(PgPool::connect(&url).await?);
        let probes: [Arc<fixture::Probe>; 2] = Default::default();
        if failure < 2 {
            providers
                .storage_failure
                .store(failure + 1, Ordering::SeqCst);
        } else {
            providers.objects.panic_stop.store(true, Ordering::SeqCst);
            if failure == 2 {
                probes[1].initialization.store(1, Ordering::SeqCst);
            }
        }
        let result = RuntimeState::initialize(
            config(root.path(), &url),
            super::identity_support::with_identity(
                Arc::new(fixture::Identity("test".into())),
                plugins(providers.clone(), probes.clone()),
            ),
        )
        .await;
        if failure == 3 {
            assert!(result?.shutdown().await.is_err());
        } else {
            assert!(result.is_err());
        }
        assert!(providers.database.pool.is_closed());
        assert_eq!(providers.database.stops.load(Ordering::SeqCst), 1);
        assert_eq!(
            providers.objects.stops.load(Ordering::SeqCst),
            usize::from(failure >= 2)
        );
        if failure >= 2 {
            assert!(
                probes
                    .iter()
                    .all(|probe| probe.stopped.load(Ordering::SeqCst) == 1)
            );
        }
    }
    Ok(())
}

#[cfg(feature = "sidecar")]
#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn sidecar_receives_the_same_checked_database_as_external_executor() -> Result<()> {
    let url = fixture::database().await?;
    let root = tempfile::tempdir()?;
    let workspace = format!("resource-sidecar-{}", Uuid::new_v4());
    let source = Uuid::new_v5(&Uuid::NAMESPACE_URL, workspace.as_bytes());
    let providers = Providers::new(PgPool::connect(&url).await?);
    let provisioner = DatabaseProvisioner::connect(&url).await?;
    binding(&providers, &provisioner, source, "development").await?;
    provisioner.shutdown().await?;
    let probes: [Arc<fixture::Probe>; 2] = Default::default();
    let state = RuntimeState::initialize(
        config(root.path(), &url),
        super::identity_support::with_identity(
            Arc::new(fixture::Identity("development".into())),
            plugins(providers.clone(), probes.clone()),
        ),
    )
    .await?;
    let bundle = Arc::new(
        package(
            root.path(),
            "https://github.com/example/resource-sidecar.git",
            false,
        )?
        .verify()?,
    );
    let context = probes[0]
        .context
        .lock()
        .await
        .clone()
        .context("缺少执行上下文")?;
    let resources = context.resources(source, "development", &bundle).await?;
    let database = resources.database.as_ref().context("缺少替代数据库")?;
    let mut transaction = database.begin().await?;
    transaction
        .execute(
            "INSERT INTO records VALUES(1,'shared-with-sidecar')",
            vec![],
        )
        .await?;
    transaction.commit().await?;
    let sidecar = SidecarExecutor::default();
    sidecar.initialize(context).await?;
    let artifact = az_plugin_development::DevArtifact {
        workspace: root.path().into(),
        source: workspace,
        content_digest: bundle.digest().into(),
        frontend: root.path().join("1.0.0/web"),
        backend: root.path().join("1.0.0/server-1"),
        backend_digest: bundle.digest().into(),
        endpoint: None,
        generation: 1,
    };
    let launch = sidecar.prepare_development(&artifact, bundle).await?;
    let path = launch
        .environment
        .get("AIO_PLUGIN_CONFIG")
        .context("缺少 Sidecar 配置")?;
    let configuration: az_plugin_contract::process::Configuration =
        serde_json::from_slice(&std::fs::read(path)?)?;
    let connection = PgPool::connect(
        configuration
            .database_url
            .as_deref()
            .context("缺少数据库授权")?,
    )
    .await?;
    let role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&connection)
        .await?;
    assert_eq!(
        role,
        sqlx::postgres::PgConnectOptions::from_str(&database.process_connection()?.direct_url)?
            .get_username()
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT value FROM records WHERE id=1")
            .fetch_one(&connection)
            .await?,
        "shared-with-sidecar"
    );
    assert!(
        sqlx::query("SELECT * FROM public.plugin_sources")
            .fetch_all(&connection)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("CREATE TABLE forbidden(id BIGINT)")
            .execute(&connection)
            .await
            .is_err()
    );
    connection.close().await;
    sidecar.shutdown().await?;
    assert!(!std::path::Path::new(path).exists());
    state.shutdown().await?;
    assert!(providers.database.pool.is_closed());
    Ok(())
}

#[cfg(feature = "wasm")]
#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL 和真实 WIT Component"]
async fn wasm_uses_replacement_storage_database_and_persistent_releases() -> Result<()> {
    let url = fixture::database().await?;
    let root = tempfile::tempdir()?;
    let source = Uuid::new_v4();
    let tenant = Uuid::new_v4().to_string();
    let git = format!("https://github.com/example/resource-wasm-{source}.git");
    let providers = Providers::new(PgPool::connect(&url).await?);
    let provisioner = DatabaseProvisioner::connect(&url).await?;
    binding(&providers, &provisioner, source, &tenant).await?;
    provisioner.shutdown().await?;
    let probes: [Arc<fixture::Probe>; 2] = Default::default();
    let state = RuntimeState::initialize(
        config(root.path(), &url),
        super::identity_support::with_identity(
            Arc::new(fixture::Identity(tenant.clone())),
            plugins(providers.clone(), probes.clone()),
        ),
    )
    .await?;
    package(root.path(), &git, true)?;
    let directory = root.path().join("1.0.0");
    let manifest = directory.join("aio-plugin.toml");
    let original = std::fs::read_to_string(&manifest)?;
    let start = original
        .find("[plugin.runtime.process]")
        .context("缺少 process 配置")?;
    let end = original.find("[plugin.frontend]").context("缺少前端配置")?;
    let text = format!("{}{}", &original[..start], &original[end..])
        .replace("artifact='server-1'", "artifact='plugin.wasm'");
    std::fs::write(&manifest, &text)?;
    std::fs::copy(
        std::env::var("AIO_TEST_HEALTHY_COMPONENT")?,
        directory.join("plugin.wasm"),
    )?;
    let bundle = Bundle::from_directory(
        &directory,
        "aio-plugin.toml",
        git.clone(),
        "a".repeat(40),
        "1.0.0".into(),
    )?;
    let context = probes[0]
        .context
        .lock()
        .await
        .clone()
        .context("缺少执行上下文")?;
    let resources = context
        .resources(source, &tenant, &bundle.verify()?)
        .await?;
    let database = resources.database.as_ref().context("缺少数据库句柄")?;
    let mut transaction = database.begin().await?;
    transaction
        .execute(
            "INSERT INTO records VALUES (1,'database-from-replacement')",
            vec![],
        )
        .await?;
    transaction.commit().await?;
    let wasm = WasmComponentExecutor::default();
    wasm.initialize(context.clone()).await?;
    wasm.activate(source, &tenant, &bundle).await?;
    let caller = az_plugin_contract::RequestContext {
        tenant_id: Some(tenant.clone()),
        request_id: "resources".into(),
        ..Default::default()
    };
    let request = |path: &str| ComponentRequest {
        method: "POST".into(),
        path: path.into(),
        query: None,
        headers: vec![],
        body: b"from-wasm".to_vec(),
    };
    let response = wasm
        .handle(
            source,
            &tenant,
            &bundle.digest,
            request("/storage"),
            caller.clone(),
        )
        .await?;
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert_eq!(
        resources
            .storage
            .as_ref()
            .context("缺少对象句柄")?
            .read("shared")
            .await?,
        b"from-wasm"
    );
    let response = wasm
        .handle(
            source,
            &tenant,
            &bundle.digest,
            request("/database"),
            caller.clone(),
        )
        .await?;
    assert_eq!(response.body, b"database-from-replacement");
    wasm.shutdown().await?;
    let restored = WasmComponentExecutor::default();
    restored.initialize(context).await?;
    restored.restore(source, &tenant, &bundle).await?;
    let response = restored
        .handle(
            source,
            &tenant,
            &bundle.digest,
            request("/storage"),
            caller.clone(),
        )
        .await?;
    assert_eq!(response.body, b"from-wasm");
    std::fs::write(&manifest, text.replace("storage=true", "storage=false"))?;
    let revoked = Bundle::from_directory(
        &directory,
        "aio-plugin.toml",
        git.clone(),
        "b".repeat(40),
        "1.1.0".into(),
    )?;
    restored.activate(source, &tenant, &revoked).await?;
    assert_eq!(
        restored
            .handle(
                source,
                &tenant,
                &revoked.digest,
                request("/storage"),
                caller.clone()
            )
            .await?
            .status,
        403
    );
    let denied_text = text
        .replace("database=true", "database=false")
        .replace("storage=true", "storage=false")
        .replace("[plugin.database]\nmigrations='migrations'\n", "");
    std::fs::write(&manifest, denied_text)?;
    let denied = Bundle::from_directory(
        &directory,
        "aio-plugin.toml",
        git,
        "b".repeat(40),
        "2.0.0".into(),
    )?;
    // 数据库迁移历史不能随撤权删除；新来源从未获得数据库能力，仍须拒绝其 WIT 调用。
    assert!(restored.activate(source, &tenant, &denied).await.is_err());
    let denied_source = Uuid::new_v4();
    restored.activate(denied_source, &tenant, &denied).await?;
    for path in ["/storage", "/database"] {
        assert_eq!(
            restored
                .handle(
                    denied_source,
                    &tenant,
                    &denied.digest,
                    request(path),
                    caller.clone()
                )
                .await?
                .status,
            403
        );
    }
    restored.shutdown().await?;
    assert!(!root.path().join("components/objects").exists());
    state.shutdown().await?;
    assert!(providers.database.pool.is_closed());
    Ok(())
}
