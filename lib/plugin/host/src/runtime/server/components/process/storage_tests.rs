//! 使用真实 Unix broker、SDK、PostgreSQL 安装目录与文件后端验证授权和数据边界。
use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use az_plugin_bundle::Bundle;
use az_plugin_contract::{process::Configuration, storage::MAX_OBJECT_BYTES};
use az_plugin_process::Host;
use az_plugin_runtime::{FileObjectStorage, ObjectStore};
use serde_json::json;
use sqlx::PgPool;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    task::JoinSet,
};
use uuid::Uuid;

use super::super::Components;
use super::{Processes, model::Start};

pub(super) struct Instance {
    pub(super) configuration: Configuration,
    client: reqwest::Client,
    pub(super) storage: Option<az_plugin_process::ObjectStorage>,
    start: Start,
    pub(super) jobs: JoinSet<()>,
}

pub(super) struct Fixture {
    root: tempfile::TempDir,
    pool: PgPool,
    pub(super) components: Arc<Components>,
    processes: Processes,
    _services: dill::Catalog,
    pub(super) backend: Arc<super::storage_probe_tests::Backend>,
}

impl Fixture {
    pub(super) async fn open() -> Result<Self> {
        // 多个验收共享测试数据库，只串行化建表，不串行化实际请求。
        static INITIALIZE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let _guard = INITIALIZE.lock().await;
        use crate::runtime::server::{plugins::*, store::PluginStore};
        let database = std::env::var("AIO_COMPONENT_TEST_DATABASE_URL")?;
        let url = reqwest::Url::parse(&database)?;
        ensure!(
            url.host_str() == Some("127.0.0.1") && url.path().ends_with("_test"),
            "只接受独立本机测试库"
        );
        let pool = PgPool::connect(&database).await?;
        PluginStore::new(pool.clone()).migrate().await?;
        let root = tempfile::tempdir()?;
        let backend = Arc::new(super::storage_probe_tests::Backend::new(
            root.path().join("objects"),
        ));
        let services = dill::Catalog::builder()
            .add::<PostgresComponentPlugin>()
            .add_value(super::storage_probe_tests::Provider(backend.clone()))
            .bind::<dyn ObjectStorageProvider, super::storage_probe_tests::Provider>()
            .build();
        let components = Components::open(
            Arc::new(PostgresComponentCatalog::open(pool.clone()).await?),
            &crate::configuration::ComponentStorage {
                database_url: database,
                root: root.path().into(),
            },
            Arc::new(super::super::load_keyring(&root.path().join("key.json"))?),
            Arc::new(super::super::tests::TestIdentity),
            services.weak_ref(),
            None,
            super::super::tests::default_executors()?,
        )
        .await?;
        let processes = Processes::new(
            Arc::downgrade(&components),
            root.path().join("process"),
            reqwest::Client::new(),
        );
        Ok(Self {
            root,
            pool,
            components,
            processes,
            _services: services,
            backend,
        })
    }

    pub(super) async fn start(
        &self,
        source: Uuid,
        tenant: &str,
        storage: bool,
    ) -> Result<Instance> {
        use crate::runtime::server::component_catalog::{
            ComponentInstallationCommit, ComponentPublication,
        };
        let path = self.root.path().join(format!("package-{source}-{storage}"));
        let git = format!("https://example.com/{source}.git");
        let bundle = package(&path, &git, storage)?;
        let verified = bundle.verify()?;
        let start = Start {
            source,
            tenant: tenant.into(),
            revision: bundle.digest.clone(),
        };
        self.components
            .catalog
            .publish(ComponentPublication {
                source,
                bundle,
                description: serde_json::from_value(json!({"label":"Storage fixture","pages":[]}))?,
                readme: String::new(),
                delivery_job: None,
                replace_publication: false,
            })
            .await?;
        let previous = self
            .components
            .catalog
            .installation(tenant, source)
            .await?
            .and_then(|entry| entry.installation)
            .map(|entry| entry.generation);
        self.components
            .catalog
            .install(ComponentInstallationCommit {
                source,
                tenant: tenant.into(),
                digest: start.revision.clone(),
                expected_generation: previous,
                excluded_digest: None,
            })
            .await?;
        let (configuration, jobs) = self
            .processes
            .configure(
                &start,
                &verified,
                &self.processes.root.join(start.id()),
                true,
            )
            .await?;
        let host = Host::from_bytes(
            &serde_json::to_vec(&configuration)?,
            self.root.path().join("unused.sock"),
        )?;
        let object_client = if storage {
            Some(host.storage()?)
        } else {
            assert!(host.storage().is_err());
            None
        };
        let client = reqwest::Client::builder()
            .unix_socket(configuration.broker_socket.as_str())
            .no_proxy()
            .timeout(Duration::from_secs(40))
            .build()?;
        Ok(Instance {
            configuration,
            client,
            storage: object_client,
            start,
            jobs,
        })
    }

    fn objects(&self, source: Uuid, tenant: &str) -> Result<ObjectStore> {
        ObjectStore::new(
            Arc::new(FileObjectStorage::new(self.root.path().join("objects"))),
            &source.to_string(),
            tenant,
        )
    }

    pub(super) async fn enabled(&self, instance: &Instance, enabled: bool) -> Result<()> {
        sqlx::query(
            "UPDATE component_installations SET enabled=$1 WHERE source_id=$2 AND tenant_id=$3",
        )
        .bind(enabled)
        .bind(instance.start.source)
        .bind(&instance.start.tenant)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

fn package(root: &Path, git: &str, storage: bool) -> Result<Bundle> {
    super::super::process_publication_tests::package(root, git, "1.0.0")?;
    let manifest = root.join("aio-plugin.toml");
    let content = std::fs::read_to_string(&manifest)?;
    std::fs::write(
        &manifest,
        format!(
            "{content}\n[plugin.capabilities]\nstorage={storage}\n[[plugin.public_routes]]\npath='/i/{{token}}'\ncontent_types=['image/png']\nmax_bytes=104857600\n"
        ),
    )?;
    Bundle::from_directory(
        root,
        "aio-plugin.toml",
        git.into(),
        "a".repeat(40),
        if storage { "2.0.0" } else { "1.0.0" }.into(),
    )
}

impl Instance {
    pub(super) fn request(&self, method: reqwest::Method, key: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, "http://localhost/storage")
            .header("x-aio-token", &self.configuration.ingress_token)
            .query(&[("key", key)])
    }

    pub(super) async fn partial_upload(&self, key: &str) -> Result<UnixStream> {
        let mut stream = UnixStream::connect(&self.configuration.broker_socket).await?;
        stream.write_all(format!("PUT /storage?key={key} HTTP/1.1\r\nHost: localhost\r\nX-Aio-Token: {}\r\nContent-Length: 5\r\nConnection: close\r\n\r\na", self.configuration.ingress_token).as_bytes()).await?;
        // 让真实连接进入等待请求体的阶段，随后再改变安装授权。
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok(stream)
    }
}

#[tokio::test]
#[ignore = "需要独立 PostgreSQL；真实 Unix broker 和原生 SDK 对象读写验收"]
async fn sidecar_storage_scopes_persists_and_revokes() -> Result<()> {
    use reqwest::Method;
    let fixture = Fixture::open().await?;
    let source = Uuid::new_v4();
    let mut first = fixture.start(source, "storage-a", true).await?;
    let mut tenant = fixture.start(source, "storage-b", true).await?;
    let mut other = fixture.start(Uuid::new_v4(), "storage-a", true).await?;
    let mut denied = fixture.start(Uuid::new_v4(), "storage-a", false).await?;
    let storage = first.storage.as_ref().context("缺少对象客户端")?.clone();
    let key = "图片/../source?tenant=other&中文";
    let payload = vec![0x85; MAX_OBJECT_BYTES];
    storage.write(key, payload.clone()).await?;
    assert_eq!(storage.read(key).await?, Some(payload.clone()));
    assert_eq!(
        fixture.objects(source, "storage-a")?.read(key).await?,
        payload
    );
    for instance in [&tenant, &other] {
        let objects = instance.storage.as_ref().context("缺少隔离对象客户端")?;
        assert_eq!(objects.read(key).await?, None);
        objects.write(key, vec![1, 2, 3]).await?;
    }
    assert_eq!(storage.read(key).await?, Some(payload));
    storage.write("empty", vec![]).await?;
    assert_eq!(storage.read("empty").await?, Some(vec![]));
    storage.remove("empty").await?;
    storage.remove("empty").await?;
    assert_eq!(storage.read("empty").await?, None);
    assert!(
        storage
            .write("large", vec![0; MAX_OBJECT_BYTES + 1])
            .await
            .is_err()
    );
    assert_eq!(
        first
            .request(Method::PUT, "large")
            .body(vec![0; MAX_OBJECT_BYTES + 1])
            .send()
            .await?
            .status(),
        413
    );
    assert_eq!(
        first
            .request(Method::PUT, "")
            .body(vec![0])
            .send()
            .await?
            .status(),
        400
    );
    assert_eq!(
        first
            .request(Method::GET, "k")
            .query(&[("tenant", "storage-b")])
            .send()
            .await?
            .status(),
        400
    );
    assert_eq!(
        first
            .request(Method::GET, "k")
            .header("x-aio-token", "forged")
            .send()
            .await?
            .status(),
        403
    );
    assert_eq!(
        denied
            .request(Method::PUT, "k")
            .body(vec![0])
            .send()
            .await?
            .status(),
        403
    );
    let mut upgraded = fixture
        .start(denied.start.source, "storage-a", true)
        .await?;
    assert_eq!(
        denied
            .request(Method::PUT, "k")
            .body(vec![0])
            .send()
            .await?
            .status(),
        403
    );
    upgraded
        .storage
        .as_ref()
        .context("缺少升级对象客户端")?
        .write("k", vec![1])
        .await?;

    let mut stream = first.partial_upload("revoked").await?;
    fixture.enabled(&first, false).await?;
    stream.write_all(b"bcde").await?;
    let mut response = String::new();
    tokio::time::timeout(Duration::from_secs(3), stream.read_to_string(&mut response)).await??;
    assert!(response.starts_with("HTTP/1.1 403"), "{response}");
    assert!(
        fixture
            .objects(source, "storage-a")?
            .read("revoked")
            .await
            .is_err()
    );
    assert!(storage.read(key).await.is_err());
    fixture.enabled(&first, true).await?;

    // 已建立的连接也必须随实例停用撤销，而非仅关闭监听 socket。
    let mut stream = first.partial_upload("stopped").await?;
    first.jobs.shutdown().await;
    let _ = stream.write_all(b"bcde").await;
    let mut response = String::new();
    let _ =
        tokio::time::timeout(Duration::from_secs(3), stream.read_to_string(&mut response)).await?;
    assert!(!response.starts_with("HTTP/1.1 204"));
    assert!(
        fixture
            .objects(source, "storage-a")?
            .read("stopped")
            .await
            .is_err()
    );
    let mut restored = fixture.start(source, "storage-a", true).await?;
    let restored_storage = restored.storage.as_ref().context("缺少恢复对象客户端")?;
    assert_eq!(
        restored_storage
            .read(key)
            .await?
            .context("重启后对象丢失")?
            .len(),
        MAX_OBJECT_BYTES
    );
    restored_storage.remove(key).await?;
    assert_eq!(restored_storage.read(key).await?, None);

    // 未发送完的上传总预算有界，不能永久占用存储并发额度。
    let mut stream = restored.partial_upload("timeout").await?;
    let mut response = String::new();
    tokio::time::timeout(
        Duration::from_secs(35),
        stream.read_to_string(&mut response),
    )
    .await??;
    assert!(response.starts_with("HTTP/1.1 408"), "{response}");
    assert!(
        fixture
            .objects(source, "storage-a")?
            .read("timeout")
            .await
            .is_err()
    );
    for instance in [
        &mut tenant,
        &mut other,
        &mut denied,
        &mut upgraded,
        &mut restored,
    ] {
        instance.jobs.shutdown().await;
    }
    fixture.components.shutdown().await?;
    fixture.pool.close().await;
    Ok(())
}

#[path = "resource_tests.rs"]
mod resource_tests;
