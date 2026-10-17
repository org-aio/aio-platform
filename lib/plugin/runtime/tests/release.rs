#![cfg(feature = "wasm")]

use az_plugin_contract::transport::{Request, Response};
use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Result, ensure};
use async_trait::async_trait;
use az_plugin_bundle::Bundle;
use az_plugin_contract::{InvocationScope, RequestContext};
use az_plugin_runtime::{ComponentEngine, ComponentSlot, HostServices, InvocationResources};
use tokio::sync::Notify;

const MANIFEST: &str = r#"
schema_version = 2
[plugin.runtime]
artifact = "backend/plugin.wasm"
host_version = ">=2026.9.11"
[plugin.frontend]
path = "frontend"
"#;

#[derive(Default)]
struct TestHost {
    hold_prepare: AtomicBool,
    preparing: Notify,
    prepared: Notify,
    activated: Notify,
    started: Notify,
    released: Notify,
}

#[async_trait]
impl HostServices for TestHost {
    async fn authorize(&self, _: &InvocationScope, permission: &str) -> Result<bool> {
        match permission {
            "prepare" if self.hold_prepare.load(Ordering::SeqCst) => {
                self.preparing.notify_one();
                self.prepared.notified().await;
            }
            "activate" => self.activated.notify_one(),
            "wait" => {
                self.started.notify_one();
                self.released.notified().await;
            }
            _ => {}
        }
        Ok(true)
    }

    async fn manage(&self, _: &InvocationScope, _: Request) -> Result<Response> {
        anyhow::bail!("测试不授予管理接口")
    }
}

fn bundle(
    bytes: &[u8],
    version: &str,
    page: &str,
) -> Result<Arc<az_plugin_bundle::VerifiedBundle>> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("backend"))?;
    fs::create_dir(root.path().join("frontend"))?;
    fs::write(root.path().join("aio-plugin.toml"), MANIFEST)?;
    fs::write(root.path().join("backend/plugin.wasm"), bytes)?;
    fs::write(root.path().join(format!("frontend/{page}")), version)?;
    let bundle = Bundle::from_directory(
        root.path(),
        "aio-plugin.toml",
        "https://example.com/fixture.git".into(),
        "a".repeat(40),
        version.into(),
    )?;
    Ok(Arc::new(Bundle::decode(&bundle.encode()?)?.verify()?))
}

fn context() -> RequestContext {
    RequestContext {
        tenant_id: Some("tenant".into()),
        request_id: "release-test".into(),
        ..Default::default()
    }
}

fn request(path: &str) -> Request {
    Request {
        method: "POST".into(),
        path: path.into(),
        query: None,
        headers: vec![],
        body: vec![0, 255, 128, 42],
    }
}

fn resources(host: &Arc<TestHost>) -> InvocationResources {
    InvocationResources {
        services: Some(host.clone()),
        ..Default::default()
    }
}

#[tokio::test]
#[ignore = "requires built WIT fixture Components"]
async fn rejected_candidates_and_cancelled_prepare_keep_active_release() -> Result<()> {
    let engine = ComponentEngine::new()?;
    let slot = Arc::new(ComponentSlot::new(
        uuid::Uuid::new_v4(),
        "tenant".into(),
        "2026.9.11".parse()?,
    )?);
    let host = Arc::new(TestHost::default());
    let healthy = fs::read(std::env::var("AIO_TEST_HEALTHY_COMPONENT")?)?;
    let first = bundle(&healthy, "1.0.0", "index.html")?;
    slot.replace(&engine, first.clone(), Default::default(), resources(&host))
        .await?;
    assert_eq!(
        slot.handle(first.digest(), request("/environment"), context())
            .await?
            .body,
        b"0"
    );
    for bad in [
        bundle(
            &fs::read(std::env::var("AIO_TEST_UNHEALTHY_COMPONENT")?)?,
            "2.0.0",
            "index.html",
        )?,
        bundle(&healthy, "2.0.0", "missing.html")?,
        bundle(b"\0asm\x0d\0\x01\0bad", "2.0.0", "index.html")?,
    ] {
        assert!(
            slot.replace(&engine, bad, Default::default(), resources(&host))
                .await
                .is_err()
        );
        assert_eq!(
            slot.snapshot().await.unwrap().bundle.digest(),
            first.digest()
        );
        assert_eq!(
            slot.handle(first.digest(), request("/echo"), context())
                .await?
                .body,
            request("/echo").body
        );
    }
    host.hold_prepare.store(true, Ordering::SeqCst);
    let candidate = bundle(&healthy, "2.0.0", "index.html")?;
    let preparation = slot.replace(&engine, candidate, Default::default(), resources(&host));
    let mut preparation = Box::pin(preparation);
    tokio::select! {
        _ = host.preparing.notified() => {},
        result = &mut preparation => anyhow::bail!("准备不应提前结束: {result:?}"),
    }
    // 不再轮询 preparation，但旧请求仍应可以完成。
    assert_eq!(
        slot.handle(first.digest(), request("/echo"), context())
            .await?
            .status,
        200
    );
    drop(preparation);
    host.hold_prepare.store(false, Ordering::SeqCst);
    slot.replace(&engine, first.clone(), Default::default(), resources(&host))
        .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires built WIT fixture Components"]
async fn replacement_drains_requests_and_revokes_old_page_revisions() -> Result<()> {
    let engine = Arc::new(ComponentEngine::new()?);
    let slot = Arc::new(ComponentSlot::new(
        uuid::Uuid::new_v4(),
        "tenant".into(),
        "2026.9.11".parse()?,
    )?);
    let host = Arc::new(TestHost::default());
    let bytes = fs::read(std::env::var("AIO_TEST_HEALTHY_COMPONENT")?)?;
    let first = bundle(&bytes, "1.0.0", "index.html")?;
    let second = bundle(&bytes, "2.0.0", "index.html")?;
    slot.replace(&engine, first.clone(), Default::default(), resources(&host))
        .await?;
    host.activated.notified().await;
    let running = {
        let slot = slot.clone();
        let digest = first.digest().to_owned();
        tokio::spawn(async move { slot.handle(&digest, request("/wait"), context()).await })
    };
    tokio::time::timeout(Duration::from_secs(5), host.started.notified()).await?;
    let updating = {
        let slot = slot.clone();
        let engine = engine.clone();
        let host = host.clone();
        let second = second.clone();
        tokio::spawn(async move {
            slot.replace(&engine, second, Default::default(), resources(&host))
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(5), host.activated.notified()).await?;
    ensure!(!updating.is_finished(), "旧请求未结束时不能替换实例");
    host.released.notify_one();
    assert_eq!(running.await??.body, request("/wait").body);
    updating.await??;
    let snapshot = slot.snapshot().await.unwrap();
    assert_eq!(snapshot.bundle.digest(), second.digest());
    assert_eq!(
        snapshot.bundle.frontend("index.html"),
        Some(b"2.0.0".as_slice())
    );
    assert!(
        slot.handle(first.digest(), request("/echo"), context())
            .await
            .is_err()
    );
    assert!(
        slot.handle(
            second.digest(),
            request("/echo"),
            RequestContext {
                tenant_id: Some("other".into()),
                ..context()
            }
        )
        .await
        .is_err()
    );
    assert_eq!(
        slot.handle(second.digest(), request("/echo"), context())
            .await?
            .status,
        200
    );
    slot.deactivate().await;
    assert!(slot.snapshot().await.is_none());
    assert!(
        slot.handle(second.digest(), request("/echo"), context())
            .await
            .is_err()
    );
    slot.replace(&engine, first.clone(), Default::default(), resources(&host))
        .await?;
    assert_eq!(
        slot.handle(first.digest(), request("/echo"), context())
            .await?
            .status,
        200
    );
    Ok(())
}

#[derive(Default)]
struct TransportHost(std::sync::Mutex<Option<Request>>);

#[async_trait]
impl HostServices for TransportHost {
    async fn authorize(&self, _: &InvocationScope, permission: &str) -> Result<bool> {
        Ok(permission == "plugin:manage")
    }

    async fn manage(&self, _: &InvocationScope, request: Request) -> Result<Response> {
        let response = Response {
            status: 202,
            headers: request.headers.clone(),
            body: request.body.clone(),
        };
        *self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("测试请求锁损坏"))? = Some(request);
        Ok(response)
    }
}

#[tokio::test]
#[ignore = "需要已构建的 WIT 测试组件"]
async fn engine_neutral_transport_survives_guest_and_host_wit_boundaries() -> Result<()> {
    let bytes = fs::read(std::env::var("AIO_TEST_HEALTHY_COMPONENT")?)?;
    let engine = ComponentEngine::new()?;
    let component = engine.compile(&bytes).await?;
    let host = Arc::new(TransportHost::default());
    let context = RequestContext {
        tenant_id: Some("tenant".into()),
        user_id: Some("user".into()),
        request_id: "transport".into(),
        ..Default::default()
    };
    let scope = InvocationScope {
        source_id: "source".into(),
        revision: "revision".into(),
        context: context.clone(),
        grants: az_plugin_contract::CapabilityGrants {
            management: true,
            ..Default::default()
        },
    };
    let mut instance = engine
        .instantiate(
            &component,
            scope,
            InvocationResources {
                services: Some(host.clone()),
                ..Default::default()
            },
        )
        .await?;
    let request = Request {
        method: "PATCH".into(),
        path: "/management".into(),
        query: Some("filter=a%2Fb&limit=2".into()),
        headers: vec![az_plugin_contract::transport::Header {
            name: "content-type".into(),
            value: "application/octet-stream".into(),
        }],
        body: vec![0, 255, 128, 42],
    };
    let response = instance.handle(request.clone(), context).await?;
    // 夹具移除管理入口前缀，再将请求转交给宿主公共路由。
    let mut delegated = request.clone();
    delegated.path = "/api/runtime/catalog".into();
    assert_eq!(
        host.0
            .lock()
            .map_err(|_| anyhow::anyhow!("测试请求锁损坏"))?
            .as_ref(),
        Some(&delegated)
    );
    assert_eq!(
        response,
        Response {
            status: 202,
            headers: request.headers,
            body: request.body
        }
    );
    Ok(())
}
