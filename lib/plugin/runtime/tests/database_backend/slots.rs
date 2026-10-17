use std::{fs, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use az_plugin_bundle::Bundle;
use az_plugin_contract::{
    CapabilityGrants, InvocationScope, RequestContext,
    transport::{Request, Response},
};
use az_plugin_runtime::{
    ComponentEngine, ComponentReleases, HostServices, InvocationResources, PersistentComponentSlot,
};
use tokio::sync::Notify;
use uuid::Uuid;

use super::{releases::SqliteReleases, sqlite::SqliteDatabase};

#[derive(Default)]
struct Services {
    started: Notify,
    activated: Notify,
    released: Notify,
}

#[async_trait::async_trait]
impl HostServices for Services {
    async fn authorize(&self, _: &InvocationScope, permission: &str) -> Result<bool> {
        if permission == "activate" {
            self.activated.notify_one();
        }
        if permission == "wait" {
            self.started.notify_one();
            self.released.notified().await;
        }
        Ok(true)
    }
    async fn manage(&self, _: &InvocationScope, _: Request) -> Result<Response> {
        anyhow::bail!("夹具不提供管理能力")
    }
}

fn bundle(version: &str) -> Result<Bundle> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("frontend"))?;
    fs::create_dir(root.path().join("migrations"))?;
    fs::write(
        root.path().join("aio-plugin.toml"),
        "schema_version=2\n[plugin.runtime]\nartifact='server.wasm'\nhost_version='>=2026.9.11'\n[plugin.frontend]\npath='frontend'\n[plugin.capabilities]\ndatabase=true\n[plugin.database]\nmigrations='migrations'\n",
    )?;
    fs::copy(
        std::env::var("AIO_TEST_HEALTHY_COMPONENT")?,
        root.path().join("server.wasm"),
    )?;
    fs::write(root.path().join("frontend/index.html"), version)?;
    fs::write(
        root.path().join("migrations/0001.sql"),
        "CREATE TABLE records(id INTEGER PRIMARY KEY,value TEXT)",
    )?;
    Bundle::from_directory(
        root.path(),
        "aio-plugin.toml",
        "https://example.com/database.git".into(),
        "a".repeat(40),
        version.into(),
    )
}

fn context(tenant: &str) -> RequestContext {
    RequestContext {
        tenant_id: Some(tenant.into()),
        ..Default::default()
    }
}
fn request(method: &str, path: &str, body: &[u8]) -> Request {
    Request {
        method: method.into(),
        path: path.into(),
        query: None,
        headers: vec![],
        body: body.into(),
    }
}
fn grants() -> CapabilityGrants {
    CapabilityGrants {
        database: true,
        ..Default::default()
    }
}

#[tokio::test]
#[ignore = "需要构建真实 WIT 组件，整个测试不选择 PostgreSQL 实现"]
async fn wasm_uses_sqlite_for_data_releases_restart_and_atomic_failure() -> Result<()> {
    let root = tempfile::tempdir()?;
    let source = Uuid::new_v4();
    let tenant = "test";
    let releases_path = root.path().join("releases.db");
    let data_path = root.path().join("tenant.db");
    let store = Arc::new(SqliteReleases::open(&releases_path).await?);
    let database = SqliteDatabase::open(&data_path, &source.to_string(), tenant).await?;
    let resources = InvocationResources {
        database: Some(database),
        services: Some(Arc::new(Services::default())),
        ..Default::default()
    };
    let engine = ComponentEngine::new()?;
    let first = bundle("1.0.0")?;
    let second = bundle("2.0.0")?;
    let third = bundle("3.0.0")?;
    let slot =
        PersistentComponentSlot::open(store.clone(), source, tenant.into(), "2026.10.17".parse()?)
            .await?;
    slot.activate(&engine, first.clone(), grants(), resources.clone())
        .await?;
    let response = slot
        .handle(
            &first.digest,
            request("PUT", "/database", b"persisted-through-wit"),
            context(tenant),
        )
        .await?;
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert_eq!(response.body, b"persisted-through-wit");
    assert!(
        slot.handle(
            &first.digest,
            request("GET", "/database", b""),
            context("other")
        )
        .await
        .is_err()
    );
    slot.activate(&engine, second.clone(), grants(), resources.clone())
        .await?;
    let generation = store.generation(source, tenant).await?;
    assert_eq!(generation, 2);
    slot.unload().await;
    drop(slot);
    drop(resources);
    store.pool.close().await;
    drop(store);

    let store = Arc::new(SqliteReleases::open(&releases_path).await?);
    let resources = InvocationResources {
        database: Some(SqliteDatabase::open(&data_path, &source.to_string(), tenant).await?),
        services: Some(Arc::new(Services::default())),
        ..Default::default()
    };
    let slot =
        PersistentComponentSlot::open(store.clone(), source, tenant.into(), "2026.10.17".parse()?)
            .await?;
    assert!(slot.restore(&engine, grants(), resources.clone()).await?);
    assert_eq!(store.generation(source, tenant).await?, generation);
    let response = slot
        .handle(
            &second.digest,
            request("GET", "/database", b""),
            context(tenant),
        )
        .await?;
    assert_eq!(response.body, b"persisted-through-wit");
    assert!(
        slot.handle(
            &first.digest,
            request("GET", "/database", b""),
            context(tenant)
        )
        .await
        .is_err()
    );

    sqlx::query("CREATE TRIGGER reject_activation BEFORE INSERT ON activations WHEN NEW.generation=3 BEGIN SELECT RAISE(ABORT,'injected commit failure'); END").execute(&store.pool).await?;
    assert!(
        slot.activate(&engine, third.clone(), grants(), resources.clone())
            .await
            .is_err()
    );
    assert_eq!(store.generation(source, tenant).await?, generation);
    assert_eq!(
        slot.snapshot()
            .await?
            .context("旧版本丢失")?
            .bundle
            .digest(),
        second.digest
    );
    assert_eq!(
        slot.handle(
            &second.digest,
            request("GET", "/database", b""),
            context(tenant)
        )
        .await?
        .status,
        200
    );
    sqlx::query("DROP TRIGGER reject_activation")
        .execute(&store.pool)
        .await?;

    // 两个独立后端连接同时以同一代数发布，只允许一个提交；审计与版本一起落盘。
    let other = SqliteReleases::open(&releases_path).await?;
    let permissions = grants();
    let (left, right) = tokio::join!(
        store.activate(source, tenant, generation, &third, &permissions),
        other.activate(source, tenant, generation, &first, &permissions)
    );
    assert_ne!(left.is_ok(), right.is_ok());
    assert_eq!(store.generation(source, tenant).await?, generation + 1);
    let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM activations")
        .fetch_one(&store.pool)
        .await?;
    assert_eq!(audits, 3);
    assert!(slot.restore(&engine, grants(), resources).await?);
    slot.deactivate().await?;
    assert!(store.stored(source, tenant).await?.is_none());
    let restart = PersistentComponentSlot::open(
        Arc::new(other),
        source,
        tenant.into(),
        "2026.10.17".parse()?,
    )
    .await?;
    assert!(
        !restart
            .restore(&engine, grants(), Default::default())
            .await?
    );
    Ok(())
}

#[tokio::test]
#[ignore = "需要构建真实 WIT 组件，验证两个独立 SQLite 后端连接"]
async fn cancelled_wasm_call_releases_backend_lease_before_another_host_publishes() -> Result<()> {
    let root = tempfile::tempdir()?;
    let source = Uuid::new_v4();
    let tenant = "test";
    let path = root.path().join("releases.db");
    let store = Arc::new(SqliteReleases::open(&path).await?);
    let database =
        SqliteDatabase::open(&root.path().join("data.db"), &source.to_string(), tenant).await?;
    let services = Arc::new(Services::default());
    let resources = InvocationResources {
        database: Some(database),
        services: Some(services.clone()),
        ..Default::default()
    };
    let engine = Arc::new(ComponentEngine::new()?);
    let first = bundle("1.0.0")?;
    let second = bundle("2.0.0")?;
    let serving = Arc::new(
        PersistentComponentSlot::open(store, source, tenant.into(), "2026.10.17".parse()?).await?,
    );
    serving
        .activate(&engine, first.clone(), grants(), resources.clone())
        .await?;
    services.activated.notified().await;
    let call = {
        let serving = serving.clone();
        let digest = first.digest.clone();
        tokio::spawn(async move {
            serving
                .handle(&digest, request("GET", "/wait", b""), context(tenant))
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(5), services.started.notified()).await?;
    let publisher = Arc::new(
        PersistentComponentSlot::open(
            Arc::new(SqliteReleases::open(&path).await?),
            source,
            tenant.into(),
            "2026.10.17".parse()?,
        )
        .await?,
    );
    let update = {
        let publisher = publisher.clone();
        let engine = engine.clone();
        let resources = resources.clone();
        tokio::spawn(async move {
            publisher
                .activate(&engine, second, grants(), resources)
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(5), services.activated.notified()).await?;
    assert!(!update.is_finished(), "旧调用持有后端读租约时不能完成发布");
    call.abort();
    assert!(call.await.err().context("调用应被取消")?.is_cancelled());
    tokio::time::timeout(Duration::from_secs(5), update).await???;
    assert!(serving.snapshot().await.is_err());
    assert!(serving.restore(&engine, grants(), resources).await?);
    serving.unload().await;
    publisher.deactivate().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "需要构建真实 WIT 组件，验证请求失败与同槽并发发布"]
async fn failed_wasm_call_does_not_deadlock_or_invalidate_a_new_release() -> Result<()> {
    let root = tempfile::tempdir()?;
    let source = Uuid::new_v4();
    let tenant = "test";
    let store = Arc::new(SqliteReleases::open(&root.path().join("releases.db")).await?);
    let database =
        SqliteDatabase::open(&root.path().join("data.db"), &source.to_string(), tenant).await?;
    let services = Arc::new(Services::default());
    let resources = InvocationResources {
        database: Some(database),
        services: Some(services.clone()),
        ..Default::default()
    };
    let engine = Arc::new(ComponentEngine::new()?);
    let first = bundle("1.0.0")?;
    let second = bundle("2.0.0")?;
    let digest = second.digest.clone();
    let slot = Arc::new(
        PersistentComponentSlot::open(store, source, tenant.into(), "2026.10.17".parse()?).await?,
    );
    slot.activate(&engine, first.clone(), grants(), resources.clone())
        .await?;
    services.activated.notified().await;
    let call = {
        let slot = slot.clone();
        tokio::spawn(async move {
            slot.handle(
                &first.digest,
                request("GET", "/wait-invalid", b""),
                context(tenant),
            )
            .await
        })
    };
    tokio::time::timeout(Duration::from_secs(5), services.started.notified()).await?;
    let update = {
        let slot = slot.clone();
        let engine = engine.clone();
        tokio::spawn(async move { slot.activate(&engine, second, grants(), resources).await })
    };
    tokio::time::timeout(Duration::from_secs(5), services.activated.notified()).await?;
    services.released.notify_one();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), call)
            .await??
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(5), update).await???;
    assert_eq!(
        slot.snapshot()
            .await?
            .context("新版本不能被失败请求清除")?
            .bundle
            .digest(),
        digest
    );
    assert_eq!(
        slot.handle(
            &digest,
            request("POST", "/echo", b"new-release"),
            context(tenant)
        )
        .await?
        .body,
        b"new-release"
    );
    slot.deactivate().await?;
    Ok(())
}
