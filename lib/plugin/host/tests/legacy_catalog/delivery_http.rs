//! 替代持久交付器经过正式 HTTP 协议发布页面包，进程重建不丢失上传与文档。
use super::*;
use anyhow::ensure;
use axum::http::HeaderMap;
use az_plugin_delivery::{BuildJob, BuildReport, DocumentImage, Documentation};
use az_plugin_host::{
    authorization::AuthorizationProvider,
    identity::{IdentityProvider, SessionContext},
    runtime::server::{RuntimeState, delivery::*},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};

struct FileDelivery(Arc<FileCatalog>, bool);
impl Plugin<RuntimeAssembly> for FileDelivery {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_builder(self.0.clone())
                .bind::<dyn PluginCatalog, FileCatalog>()
                .bind::<dyn PluginDocumentation, FileCatalog>();
            if self.1 {
                assembly
                    .services
                    .bind::<dyn DeliveryService, FileCatalog>()
                    .bind::<dyn DeliverySources, FileCatalog>();
            }
            Ok(())
        })
    }
}
struct ReaderIdentity;
#[async_trait::async_trait]
impl IdentityProvider for ReaderIdentity {
    async fn authenticate(&self, headers: &HeaderMap) -> Result<Option<SessionContext>> {
        let mut session = Identity("tenant".into()).authenticate(headers).await?;
        if headers.contains_key("x-test-reader")
            && let Some(session) = session.as_mut()
        {
            session.permissions.clear();
        }
        Ok(session)
    }
    async fn member_active(&self, tenant: &str, user: &str) -> Result<bool> {
        Identity("tenant".into()).member_active(tenant, user).await
    }
    async fn session_active(&self, session: &str, tenant: &str, user: &str) -> Result<bool> {
        Identity("tenant".into())
            .session_active(session, tenant, user)
            .await
    }
}
#[async_trait::async_trait]
impl AuthorizationProvider for ReaderIdentity {
    async fn can_publish(&self, session: &SessionContext) -> Result<bool> {
        Ok(session.permissions.iter().any(|p| p == "plugin:manage"))
    }
}
pub(super) fn selection(
    catalog: Arc<FileCatalog>,
    delivery: bool,
) -> Vec<DynPlugin<RuntimeAssembly>> {
    let mut plugins: Vec<DynPlugin<RuntimeAssembly>> = vec![
        Arc::new(ManagementPlugin),
        Arc::new(RepositoryPlugin),
        Arc::new(FrontendPlugin),
        Arc::new(FileDelivery(catalog, delivery)),
        Arc::new(DocumentationPlugin),
    ];
    if delivery {
        plugins.push(Arc::new(DeliveryPlugin));
    }
    identity_support::with_identity(Arc::new(ReaderIdentity), plugins)
}
pub(super) fn build_request(host: &Host, route: &str) -> Result<reqwest::RequestBuilder> {
    let token = std::env::var("AIO_DELIVERY_TOKEN")?;
    ensure!(token.len() >= 32, "隔离测试需要构建凭据");
    Ok(host
        .client
        .post(format!("{}/api/internal/delivery{route}", host.base))
        .bearer_auth(token))
}
pub(super) async fn claim(host: &Host) -> Result<BuildJob> {
    build_request(host, "/claim")?
        .send()
        .await?
        .error_for_status()?
        .json::<Option<BuildJob>>()
        .await?
        .context("没有构建任务")
}
pub(super) fn artifact(job: &BuildJob) -> Result<PluginPackage> {
    PluginPackage::new(job.git.clone(), job.version.clone(), Some(job.source_revision.clone()),
        "[plugin.runtime]\nkind='page-definition'\nartifact='pages.json'\n[plugin.marketplace]\ntitle='Delivered'\nsummary='File delivery'\nlicense='MIT'\ntags=['test']\n[[plugin.subplugins]]\nid='delivered'\npages=['delivered-page']\n".into(),
        br#"[{"id":"delivered-page","label":"Delivered","scene":{"id":"tools","label":"Tools"},"body":{"kind":"text","title":"Ready","content":"Built outside PostgreSQL"}}]"#,
        Default::default())
}
pub(super) async fn upload(
    host: &Host,
    job: &BuildJob,
    package: &PluginPackage,
) -> Result<reqwest::Response> {
    Ok(build_request(host, &format!("/jobs/{}/package", job.id))?
        .header("x-aio-build-lease", &job.lease)
        .header("content-type", PACKAGE_CONTENT_TYPE)
        .body(package.encode()?)
        .send()
        .await?)
}

#[tokio::test]
#[ignore = "需要独立 AIO_DELIVERY_TOKEN"]
async fn replacement_delivery_publishes_recovers_and_serves_documents_without_postgres()
-> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("delivery.json");
    let git = "https://github.com/example/file-delivery.git";
    let catalog = FileCatalog::open(&path);
    catalog.enqueue(git, &"a".repeat(40)).await?;
    let host = Host::start_selected(
        root.path(),
        "invalid database",
        selection(catalog.clone(), true),
    )
    .await?;
    #[cfg(feature = "postgres")]
    assert!(host.state.service::<sqlx::PgPool>().is_err());
    #[cfg(feature = "postgres")]
    assert!(host.state.service::<PluginStore>().is_err());
    assert_eq!(
        host.client
            .post(format!("{}/api/internal/delivery/claim", host.base))
            .send()
            .await?
            .status(),
        401
    );
    let (a, b) = tokio::try_join!(
        build_request(&host, "/claim")?.send(),
        build_request(&host, "/claim")?.send()
    )?;
    let a: Option<BuildJob> = a.error_for_status()?.json().await?;
    let b: Option<BuildJob> = b.error_for_status()?.json().await?;
    assert_ne!(a.is_some(), b.is_some(), "不能同时领取两次");
    let first = a.or(b).context("领取缺失")?;
    assert_eq!(
        build_request(&host, &format!("/jobs/{}/heartbeat", first.id))?
            .json(&json!({"lease":"wrong"}))
            .send()
            .await?
            .status(),
        409
    );
    build_request(&host, &format!("/jobs/{}/defer", first.id))?
        .json(&json!({"lease":first.lease,"error":"try again"}))
        .send()
        .await?
        .error_for_status()?;
    let job = claim(&host).await?;
    assert_ne!(job.lease, first.lease);
    let package = artifact(&job)?;
    assert!(!upload(&host, &first, &package).await?.status().is_success());
    upload(&host, &job, &package).await?.error_for_status()?;
    let details = format!("/api/runtime/marketplace/{}/details", package.rev);
    assert_eq!(get(&host, &details).await?["data"]["versions"], json!([]));
    host.stop().await?;
    drop(catalog);

    let catalog = FileCatalog::open(&path);
    let mut host = Host::start_selected(
        root.path(),
        "invalid database",
        selection(catalog.clone(), true),
    )
    .await?;
    let retained: Vec<i64> = build_request(&host, "/retained-jobs")?
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(retained, vec![job.id]);
    build_request(&host, &format!("/jobs/{}/heartbeat", job.id))?
        .json(&json!({"lease":job.lease}))
        .send()
        .await?
        .error_for_status()?;
    let image = b"\x89PNG\r\n\x1a\n\x00\xff";
    let mut documentation = Documentation {
        readme: "# Delivered".into(),
        images: Default::default(),
    };
    documentation.images.insert(
        "截图.png".into(),
        DocumentImage {
            content_type: "image/png".into(),
            content_base64: STANDARD.encode(image),
        },
    );
    let report = BuildReport {
        lease: job.lease.clone(),
        error: None,
        documentation: documentation.clone(),
    };
    let completion = format!("/jobs/{}/complete", job.id);
    catalog.fail_commit.store(true, Ordering::SeqCst);
    assert!(
        !build_request(&host, &completion)?
            .json(&report)
            .send()
            .await?
            .status()
            .is_success()
    );
    assert_eq!(get(&host, &details).await?["data"]["versions"], json!([]));
    assert_eq!(catalog.builds(git).await?[0]["state"], "uploaded");
    build_request(&host, &completion)?
        .json(&report)
        .send()
        .await?
        .error_for_status()?;
    let data = get(&host, &details).await?;
    assert_eq!(data["data"]["readme"], "# Delivered");
    assert_eq!(data["data"]["builds"][0]["state"], "active");
    let reader: Value = host
        .client
        .get(format!("{}{details}", host.base))
        .header("x-test-reader", "1")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(reader["data"]["builds"], json!([]));
    let bytes = host
        .client
        .get(format!(
            "{}/api/runtime/packages/{}",
            host.base, package.rev
        ))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    assert_eq!(bytes.as_ref(), package.encode()?.as_slice());
    success(
        host.post(
            "/api/runtime/plugins/install",
            json!({"git":git,"rev":package.rev}),
        )
        .await?,
    )
    .await?;
    assert_eq!(
        get(&host, "/api/runtime/catalog").await?["data"]["pages"][0]["id"],
        "delivered-page"
    );
    let image_url = format!(
        "{}/api/runtime/marketplace/{}/images/截图.png",
        host.base, package.rev
    );
    let response = host
        .client
        .get(&image_url)
        .send()
        .await?
        .error_for_status()?;
    assert_eq!(response.headers()["content-type"], "image/png");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.bytes().await?.as_ref(), image);
    assert_eq!(
        reqwest::Client::new()
            .get(&image_url)
            .send()
            .await?
            .status(),
        401
    );
    catalog
        .save(
            &package.rev,
            &Documentation {
                readme: "changed".into(),
                images: Default::default(),
            },
        )
        .await?;
    assert_eq!(get(&host, &details).await?["data"]["readme"], "# Delivered");
    // 文档不需要交付 HTTP 入口；随后替换整个实例，验证实际 Dill 对象发生变化。
    host.reconfigure(selection(catalog.clone(), false)).await?;
    assert!(host.state.service::<dyn DeliveryService>().is_err());
    assert_eq!(get(&host, &details).await?["data"]["builds"], json!([]));
    assert_eq!(build_request(&host, "/claim")?.send().await?.status(), 404);
    assert_eq!(get(&host, &details).await?["data"]["readme"], "# Delivered");
    let replacement = FileCatalog::open(&path);
    host.reconfigure(selection(replacement.clone(), true))
        .await?;
    assert!(Arc::ptr_eq(
        &host.state.service::<dyn DeliveryService>()?,
        &(replacement.clone() as Arc<dyn DeliveryService>)
    ));
    replacement.enqueue(git, &"b".repeat(40)).await?;
    let obsolete = claim(&host).await?;
    replacement.enqueue(git, &"c".repeat(40)).await?;
    assert!(
        !upload(&host, &obsolete, &artifact(&obsolete)?)
            .await?
            .status()
            .is_success()
    );
    let failed = claim(&host).await?;
    build_request(&host, &format!("/jobs/{}/complete", failed.id))?
        .json(&BuildReport {
            lease: failed.lease,
            error: Some("compiler failed".into()),
            documentation: Documentation::default(),
        })
        .send()
        .await?
        .error_for_status()?;
    let retry = format!(
        "{}/api/runtime/delivery/jobs/{}/retry",
        host.base, failed.id
    );
    assert_eq!(
        host.client
            .post(&retry)
            .header("x-test-reader", "1")
            .send()
            .await?
            .status(),
        403
    );
    host.client.post(&retry).send().await?.error_for_status()?;
    assert_eq!(claim(&host).await?.id, failed.id);
    host.stop().await?;
    let host = Host::start_selected(
        root.path(),
        "invalid database",
        selection(FileCatalog::open(&path), false),
    )
    .await?;
    assert_eq!(get(&host, &details).await?["data"]["readme"], "# Delivered");
    assert_eq!(
        get(&host, "/api/runtime/catalog").await?["data"]["pages"][0]["id"],
        "delivered-page"
    );
    host.stop().await
}

#[tokio::test]
async fn delivery_requires_one_provider_and_documents_can_run_independently() -> Result<()> {
    let root = tempfile::tempdir()?;
    let missing =
        identity_support::with_identity(Arc::new(ReaderIdentity), vec![Arc::new(DeliveryPlugin)]);
    assert!(
        RuntimeState::initialize(
            super::super::fixture::config(root.path(), "invalid", "http://localhost"),
            missing
        )
        .await
        .is_err()
    );
    let catalog = FileCatalog::open(&root.path().join("catalog.json"));
    let mut duplicate = selection(catalog.clone(), true);
    struct Duplicate(Arc<FileCatalog>);
    impl Plugin<RuntimeAssembly> for Duplicate {
        fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
            Box::pin(async move {
                assembly
                    .services
                    .add_builder(self.0.clone())
                    .bind::<dyn DeliveryService, FileCatalog>();
                Ok(())
            })
        }
    }
    duplicate.push(Arc::new(Duplicate(catalog)));
    assert!(
        RuntimeState::initialize(
            super::super::fixture::config(root.path(), "invalid", "http://localhost"),
            duplicate
        )
        .await
        .is_err()
    );
    let documents = identity_support::with_identity(
        Arc::new(ReaderIdentity),
        vec![Arc::new(DocumentationPlugin)],
    );
    let host = Host::start_selected(root.path(), "invalid database", documents).await?;
    #[cfg(feature = "postgres")]
    assert!(host.state.service::<sqlx::PgPool>().is_err());
    assert_eq!(
        get(&host, "/api/runtime/marketplace/missing/details").await?["data"]["versions"],
        json!([])
    );
    assert_eq!(
        host.client
            .get(format!(
                "{}/api/runtime/marketplace/missing/images/test.png",
                host.base
            ))
            .send()
            .await?
            .status(),
        404
    );
    host.stop().await
}
