use std::{any::Any, sync::Arc};

use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

pub(crate) const MAX_OBJECT_BYTES: usize = 16 * 1024 * 1024;

/// 运行时生成的对象地址；后端只能消费已隔离的命名空间和已校验键的摘要。
pub struct ObjectKey {
    namespace: String,
    digest: String,
}

impl ObjectKey {
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// 对象读写实现，由宿主插件注入；所有执行器通过 ObjectStore 共享隔离与配额。
#[async_trait::async_trait]
pub trait ObjectStorage: Any + Send + Sync {
    async fn read(&self, key: &ObjectKey) -> Result<Vec<u8>>;
    async fn write(&self, key: &ObjectKey, bytes: &[u8]) -> Result<()>;
    async fn remove(&self, key: &ObjectKey) -> Result<()>;
    /// 释放本次宿主拥有的连接或任务，保留对象；无外部资源的实现无需处理。
    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

/// 绑定来源和租户的能力句柄；校验逻辑不交给可替换后端。
#[derive(Clone)]
pub struct ObjectStore {
    backend: Arc<dyn ObjectStorage>,
    namespace: String,
    gate: Arc<Mutex<()>>,
    source: String,
    tenant: String,
}

impl ObjectStore {
    pub fn new(backend: Arc<dyn ObjectStorage>, source: &str, tenant: &str) -> Result<Self> {
        ensure!(
            !source.is_empty() && !tenant.is_empty() && tenant.len() <= 128,
            "对象存储归属无效"
        );
        let namespace = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(source, tenant))?)
        );
        Ok(Self {
            backend,
            namespace,
            gate: Arc::new(Mutex::new(())),
            source: source.into(),
            tenant: tenant.into(),
        })
    }

    pub(crate) fn matches(&self, source: &str, tenant: &str) -> bool {
        self.source == source && self.tenant == tenant
    }

    fn key(&self, key: &str) -> Result<ObjectKey> {
        ensure!(!key.is_empty() && key.len() <= 1024, "对象键长度无效");
        Ok(ObjectKey {
            namespace: self.namespace.clone(),
            digest: format!("{:x}", Sha256::digest(key.as_bytes())),
        })
    }

    pub async fn read(&self, key: &str) -> Result<Vec<u8>> {
        let key = self.key(key)?;
        let _guard = self.gate.lock().await;
        let bytes = self.backend.read(&key).await?;
        ensure!(bytes.len() <= MAX_OBJECT_BYTES, "对象超过 16 MiB 配额");
        Ok(bytes)
    }

    pub async fn write(&self, key: &str, bytes: &[u8]) -> Result<()> {
        ensure!(bytes.len() <= MAX_OBJECT_BYTES, "对象超过 16 MiB 配额");
        let key = self.key(key)?;
        let _guard = self.gate.lock().await;
        self.backend.write(&key, bytes).await
    }

    pub async fn remove(&self, key: &str) -> Result<()> {
        let key = self.key(key)?;
        let _guard = self.gate.lock().await;
        self.backend.remove(&key).await
    }
}
