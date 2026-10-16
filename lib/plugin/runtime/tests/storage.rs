use std::{collections::BTreeMap, sync::Arc};

use anyhow::{Context, Result};
use az_plugin_runtime::{
    FileObjectStorage, InvocationResources, ObjectKey, ObjectStorage, ObjectStore,
};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

#[derive(Default)]
struct MemoryStorage {
    values: Mutex<BTreeMap<(String, String), Vec<u8>>>,
}

#[async_trait::async_trait]
impl ObjectStorage for MemoryStorage {
    async fn read(&self, key: &ObjectKey) -> Result<Vec<u8>> {
        self.values
            .lock()
            .await
            .get(&(key.namespace().into(), key.digest().into()))
            .cloned()
            .context("测试对象不存在")
    }
    async fn write(&self, key: &ObjectKey, bytes: &[u8]) -> Result<()> {
        self.values.lock().await.insert(
            (key.namespace().into(), key.digest().into()),
            bytes.to_vec(),
        );
        Ok(())
    }
    async fn remove(&self, key: &ObjectKey) -> Result<()> {
        self.values
            .lock()
            .await
            .remove(&(key.namespace().into(), key.digest().into()));
        Ok(())
    }
}

#[tokio::test]
async fn replacement_preserves_scope_and_enforces_limits_outside_backend() -> Result<()> {
    let backend = Arc::new(MemoryStorage::default());
    let first = ObjectStore::new(backend.clone(), "source", "first")?;
    let second = ObjectStore::new(backend.clone(), "source", "second")?;
    let foreign = ObjectStore::new(backend.clone(), "other", "first")?;
    first.write("../../private", b"payload").await?;
    assert_eq!(first.read("../../private").await?, b"payload");
    assert!(second.read("../../private").await.is_err());
    assert!(foreign.read("../../private").await.is_err());
    let resources = InvocationResources {
        storage: Some(first.clone()),
        ..Default::default()
    };
    resources.validate_scope("source", "first")?;
    assert!(resources.validate_scope("source", "second").is_err());
    assert!(resources.validate_scope("other", "first").is_err());
    for key in [String::new(), "a".repeat(1025)] {
        assert!(first.write(&key, b"invalid").await.is_err());
        assert!(first.read(&key).await.is_err());
        assert!(first.remove(&key).await.is_err());
    }
    let oversized = vec![0; 16 * 1024 * 1024 + 1];
    assert!(first.write("oversized", &oversized).await.is_err());
    let mut values = backend.values.lock().await;
    assert_eq!(values.len(), 1);
    // 即使后端误返回超限对象，执行器也不能取得该数据。
    *values.values_mut().next().context("缺少测试对象")? = oversized;
    drop(values);
    assert!(first.read("../../private").await.is_err());
    first.remove("../../private").await?;
    first.remove("../../private").await?;
    assert!(backend.values.lock().await.is_empty());
    Ok(())
}

#[tokio::test]
async fn file_backend_reads_existing_layout_and_rejects_symbolic_links() -> Result<()> {
    let root = tempfile::tempdir()?;
    let namespace = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&("source", "first"))?)
    );
    let directory = root.path().join(&namespace);
    std::fs::create_dir_all(&directory)?;
    let key = "../../private";
    let path = directory.join(format!("{:x}", Sha256::digest(key.as_bytes())));
    std::fs::write(&path, b"retained")?;
    let backend = Arc::new(FileObjectStorage::new(root.path().into()));
    let store = ObjectStore::new(backend.clone(), "source", "first")?;
    assert_eq!(store.read(key).await?, b"retained");
    store.write(key, b"replaced").await?;
    assert_eq!(std::fs::read(&path)?, b"replaced");
    let other = ObjectStore::new(backend, "source", "other")?;
    assert!(other.read(key).await.is_err());
    store.remove(key).await?;
    assert!(!path.exists());
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir()?;
        let secret = outside.path().join("secret");
        std::fs::write(&secret, b"not exposed")?;
        std::os::unix::fs::symlink(&secret, &path)?;
        assert!(store.read(key).await.is_err());
        store.remove(key).await?;
        assert!(secret.exists());
        std::fs::remove_dir(&directory)?;
        std::os::unix::fs::symlink(outside.path(), &directory)?;
        assert!(store.write(key, b"escaped").await.is_err());
        assert!(store.read(key).await.is_err());
        assert!(store.remove(key).await.is_err());
    }
    Ok(())
}
