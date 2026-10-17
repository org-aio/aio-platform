use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use az_plugin_runtime::{FileObjectStorage, ObjectKey, ObjectStorage};
use tokio::sync::Semaphore;

use super::storage_tests::Fixture;

/// 仍写入真实文件，只在提供者边界注入延迟与故障。
pub(super) struct Backend {
    files: FileObjectStorage,
    fail: AtomicBool,
    pause: AtomicBool,
    entered: Semaphore,
    resume: Semaphore,
}

impl Backend {
    pub fn new(path: PathBuf) -> Self {
        Self {
            files: FileObjectStorage::new(path),
            fail: AtomicBool::new(false),
            pause: AtomicBool::new(false),
            entered: Semaphore::new(0),
            resume: Semaphore::new(0),
        }
    }

    fn available(&self) -> Result<()> {
        ensure!(
            !self.fail.load(Ordering::SeqCst),
            "private-backend-location"
        );
        Ok(())
    }
}

#[async_trait::async_trait]
impl ObjectStorage for Backend {
    async fn read(&self, key: &ObjectKey) -> Result<Vec<u8>> {
        self.available()?;
        if self.pause.load(Ordering::SeqCst) {
            self.entered.add_permits(1);
            self.resume.acquire().await?.forget();
        }
        self.files.read(key).await
    }
    async fn write(&self, key: &ObjectKey, bytes: &[u8]) -> Result<()> {
        self.available()?;
        self.files.write(key, bytes).await
    }
    async fn remove(&self, key: &ObjectKey) -> Result<()> {
        self.available()?;
        self.files.remove(key).await
    }
}

#[derive(Clone)]
pub(super) struct Provider(pub Arc<Backend>);

#[async_trait::async_trait]
impl crate::runtime::server::plugins::ObjectStorageProvider for Provider {
    async fn open(
        &self,
        _: &crate::configuration::ComponentStorage,
    ) -> Result<Arc<dyn ObjectStorage>> {
        Ok(self.0.clone())
    }
}

#[tokio::test]
#[ignore = "需要独立 PostgreSQL；替换对象后端并注入延迟和故障"]
async fn sidecar_storage_bounds_queue_rechecks_authority_and_cancels_backend() -> Result<()> {
    use reqwest::Method;
    let fixture = Fixture::open().await?;
    let mut instance = fixture
        .start(uuid::Uuid::new_v4(), "storage-probe", true)
        .await?;
    let storage = instance.storage.as_ref().context("缺少对象客户端")?.clone();
    storage.write("key", vec![7]).await?;
    fixture.backend.fail.store(true, Ordering::SeqCst);
    for method in [Method::GET, Method::PUT, Method::DELETE] {
        let response = instance.request(method, "key").body(vec![8]).send().await?;
        assert_eq!(response.status(), 502);
        assert!(!response.text().await?.contains("private-backend-location"));
    }
    assert!(storage.read("key").await.is_err());
    fixture.backend.fail.store(false, Ordering::SeqCst);
    assert_eq!(storage.read("key").await?, Some(vec![7]));

    fixture.backend.pause.store(true, Ordering::SeqCst);
    let reader = storage.clone();
    let reading = tokio::spawn(async move { reader.read("key").await });
    tokio::time::timeout(Duration::from_secs(3), fixture.backend.entered.acquire())
        .await??
        .forget();
    let writer = storage.clone();
    let writing = tokio::spawn(async move { writer.write("key", vec![9]).await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    fixture.enabled(&instance, false).await?;
    fixture.backend.resume.add_permits(1);
    assert!(reading.await?.is_err());
    assert!(writing.await?.is_err());
    fixture.backend.pause.store(false, Ordering::SeqCst);
    fixture.enabled(&instance, true).await?;
    assert_eq!(storage.read("key").await?, Some(vec![7]));

    let mut uploads = Vec::new();
    for index in 0..8 {
        uploads.push(instance.partial_upload(&format!("slow-{index}")).await?);
    }
    assert_eq!(
        instance.request(Method::GET, "key").send().await?.status(),
        429
    );
    drop(uploads);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if storage.read("key").await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;

    storage
        .write(
            "large",
            vec![1; az_plugin_contract::storage::MAX_OBJECT_BYTES],
        )
        .await?;
    let mut downloads = Vec::new();
    for _ in 0..8 {
        downloads.push(
            instance
                .request(Method::GET, "large")
                .send()
                .await?
                .error_for_status()?,
        );
    }
    assert_eq!(
        instance.request(Method::GET, "key").send().await?.status(),
        429
    );
    drop(downloads);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if storage.read("key").await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;

    fixture.backend.pause.store(true, Ordering::SeqCst);
    let reading = tokio::spawn(async move { storage.read("key").await });
    tokio::time::timeout(Duration::from_secs(3), fixture.backend.entered.acquire())
        .await??
        .forget();
    instance.jobs.shutdown().await;
    assert!(
        tokio::time::timeout(Duration::from_secs(3), reading)
            .await??
            .is_err()
    );
    fixture.components.shutdown().await?;
    Ok(())
}
