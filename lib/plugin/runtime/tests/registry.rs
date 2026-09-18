use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use anyhow::Result;
use async_trait::async_trait;
use az_plugin_bundle::Bundle;
use az_plugin_contract::{InvocationScope, RequestContext};
use az_plugin_runtime::{
    ComponentEngine, DatabaseProvisioner, HostServices, InvocationResources,
    bindings::aio::plugin::transport::{Request, Response},
};

#[derive(Default)]
struct Services {
    started: tokio::sync::Notify,
    released: tokio::sync::Notify,
    activated: tokio::sync::Notify,
    wait_calls: AtomicUsize,
}

#[async_trait]
impl HostServices for Services {
    async fn authorize(&self, _: &InvocationScope, permission: &str) -> Result<bool> {
        if permission == "activate" {
            self.activated.notify_one();
        }
        if permission == "wait" {
            self.wait_calls.fetch_add(1, Ordering::SeqCst);
            self.started.notify_one();
            self.released.notified().await;
        }
        Ok(true)
    }
    async fn manage(&self, _: &InvocationScope, _: Request) -> Result<Response> {
        anyhow::bail!("未授予管理能力")
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL and built WIT fixture Components"]
async fn cancelled_persistent_request_recovers_for_follow_up_without_replay() -> Result<()> {
    let provisioner =
        DatabaseProvisioner::connect(&std::env::var("AIO_TEST_DATABASE_URL")?).await?;
    let engine = ComponentEngine::new()?;
    let source = uuid::Uuid::new_v4();
    let tenant = source.to_string();
    let first = bundle(
        &fs::read(std::env::var("AIO_TEST_HEALTHY_COMPONENT")?)?,
        "1.0.0",
        "index.html",
    )?;
    let slot = provisioner
        .component_slot(source, tenant.clone(), "2026.9.12".parse()?)
        .await?;
    let host = Arc::new(Services::default());
    let resources = InvocationResources {
        services: Some(host.clone()),
        ..Default::default()
    };
    slot.activate(
        &engine,
        first.clone(),
        Default::default(),
        resources.clone(),
    )
    .await?;
    let cancelled = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        slot.handle(&first.digest, path_request("/wait"), context(&tenant)),
    )
    .await;
    assert!(cancelled.is_err(), "请求应被宿主服务阻塞并超时");
    assert_eq!(host.wait_calls.load(Ordering::SeqCst), 1);
    assert!(
        slot.handle(&first.digest, path_request("/echo"), context(&tenant))
            .await
            .is_err(),
        "取消后的槽必须先恢复，不能继续复用失效实例"
    );
    slot.restore(&engine, Default::default(), resources).await?;
    let response = slot
        .handle(&first.digest, path_request("/echo"), context(&tenant))
        .await?;
    assert_eq!(response.status, 200);
    assert_eq!(host.wait_calls.load(Ordering::SeqCst), 1);
    Ok(())
}

fn bundle(bytes: &[u8], version: &str, entry: &str) -> Result<Bundle> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("backend"))?;
    fs::create_dir(root.path().join("frontend"))?;
    fs::write(
        root.path().join("aio-plugin.toml"),
        "schema_version=2\n[plugin.runtime]\nartifact='backend/plugin.wasm'\nhost_version='>=2026.9.11'\n[plugin.frontend]\npath='frontend'\n",
    )?;
    fs::write(root.path().join("backend/plugin.wasm"), bytes)?;
    fs::write(root.path().join(format!("frontend/{entry}")), version)?;
    Bundle::from_directory(
        root.path(),
        "aio-plugin.toml",
        "https://example.com/registry.git".into(),
        "a".repeat(40),
        version.into(),
    )
}

fn resources() -> InvocationResources {
    InvocationResources {
        services: Some(Arc::new(Services::default())),
        ..Default::default()
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL and built WIT fixture Components"]
async fn activation_drains_requests_served_by_another_slot() -> Result<()> {
    let provisioner =
        DatabaseProvisioner::connect(&std::env::var("AIO_TEST_DATABASE_URL")?).await?;
    let engine = Arc::new(ComponentEngine::new()?);
    let source = uuid::Uuid::new_v4();
    let tenant = source.to_string();
    let first = bundle(
        &fs::read(std::env::var("AIO_TEST_HEALTHY_COMPONENT")?)?,
        "1.0.0",
        "index.html",
    )?;
    let second = bundle(
        &fs::read(std::env::var("AIO_TEST_HEALTHY_COMPONENT")?)?,
        "2.0.0",
        "index.html",
    )?;
    let publisher = Arc::new(
        provisioner
            .component_slot(source, tenant.clone(), "2026.9.12".parse()?)
            .await?,
    );
    publisher
        .activate(&engine, first.clone(), Default::default(), resources())
        .await?;
    let host = Arc::new(Services::default());
    let grants = InvocationResources {
        services: Some(host.clone()),
        ..Default::default()
    };
    let serving = Arc::new(
        provisioner
            .component_slot(source, tenant.clone(), "2026.9.12".parse()?)
            .await?,
    );
    serving
        .restore(&engine, Default::default(), grants.clone())
        .await?;
    host.activated.notified().await;
    let running = {
        let serving = serving.clone();
        let tenant = tenant.clone();
        tokio::spawn(async move {
            serving
                .handle(
                    &first.digest,
                    Request {
                        path: "/wait".into(),
                        ..request()
                    },
                    context(&tenant),
                )
                .await
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), host.started.notified()).await?;
    let updating = {
        let publisher = publisher.clone();
        let engine = engine.clone();
        tokio::spawn(async move {
            publisher
                .activate(&engine, second, Default::default(), grants)
                .await
        })
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        host.activated.notified(),
    )
    .await?;
    assert!(!updating.is_finished());
    host.released.notify_one();
    assert_eq!(running.await??.body, request().body);
    updating.await??;
    publisher.deactivate().await?;
    serving.unload().await;
    Ok(())
}

fn context(tenant: &str) -> RequestContext {
    RequestContext {
        tenant_id: Some(tenant.into()),
        request_id: "registry-test".into(),
        ..Default::default()
    }
}

fn request() -> Request {
    Request {
        method: "POST".into(),
        path: "/echo".into(),
        query: None,
        headers: vec![],
        body: vec![0, 255, 128, 42],
    }
}

fn path_request(path: &str) -> Request {
    Request {
        path: path.into(),
        ..request()
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL and built WIT fixture Components"]
async fn durable_activation_restores_and_fences_other_hosts() -> Result<()> {
    let provisioner =
        DatabaseProvisioner::connect(&std::env::var("AIO_TEST_DATABASE_URL")?).await?;
    let engine = ComponentEngine::new()?;
    let source = uuid::Uuid::new_v4();
    let tenant = format!("registry-{source}");
    let slot = provisioner
        .component_slot(source, tenant.clone(), "2026.9.12".parse()?)
        .await?;
    let healthy = fs::read(std::env::var("AIO_TEST_HEALTHY_COMPONENT")?)?;
    let first = bundle(&healthy, "1.0.0", "index.html")?;
    slot.activate(&engine, first.clone(), Default::default(), resources())
        .await?;
    let generation = slot.stored().await?.unwrap().generation;
    assert_eq!(
        slot.handle(&first.digest, request(), context(&tenant))
            .await?
            .body,
        request().body
    );
    slot.unload().await;
    assert!(slot.snapshot().await?.is_none());
    assert!(
        slot.restore(&engine, Default::default(), resources())
            .await?
    );
    assert_eq!(slot.stored().await?.unwrap().generation, generation);
    assert_eq!(
        slot.snapshot().await?.unwrap().bundle.digest(),
        first.digest
    );

    let other = provisioner
        .component_slot(source, tenant.clone(), "2026.9.12".parse()?)
        .await?;
    assert!(
        other
            .restore(&engine, Default::default(), resources())
            .await?
    );
    for bad in [
        bundle(
            &fs::read(std::env::var("AIO_TEST_UNHEALTHY_COMPONENT")?)?,
            "2.0.0",
            "index.html",
        )?,
        bundle(&healthy, "2.0.0", "missing.html")?,
    ] {
        assert!(
            slot.activate(&engine, bad, Default::default(), resources())
                .await
                .is_err()
        );
        assert_eq!(slot.stored().await?.unwrap().bundle.digest, first.digest);
    }
    let second = bundle(&healthy, "2.0.0", "index.html")?;
    slot.activate(&engine, second.clone(), Default::default(), resources())
        .await?;
    assert!(
        other
            .handle(&first.digest, request(), context(&tenant))
            .await
            .is_err()
    );
    assert!(other.snapshot().await.is_err());
    assert!(
        other
            .restore(&engine, Default::default(), resources())
            .await?
    );
    assert_eq!(
        other
            .handle(&second.digest, request(), context(&tenant))
            .await?
            .body,
        request().body
    );
    assert!(
        other
            .handle(&second.digest, request(), context("another-tenant"))
            .await
            .is_err()
    );
    let isolated = provisioner
        .component_slot(source, "another-tenant".into(), "2026.9.12".parse()?)
        .await?;
    assert!(isolated.stored().await?.is_none());
    slot.deactivate().await?;
    assert!(slot.stored().await?.is_none());
    assert!(
        other
            .handle(&second.digest, request(), context(&tenant))
            .await
            .is_err()
    );
    other.unload().await;
    assert!(
        !other
            .restore(&engine, Default::default(), resources())
            .await?
    );
    Ok(())
}
