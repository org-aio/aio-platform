use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{ObjectKey, ObjectStorage};
use az_plugin_contract::storage::MAX_OBJECT_BYTES;

/// 默认本地对象后端；目录和文件摘要沿用既有格式，切换接口不迁移数据。
pub struct FileObjectStorage {
    root: PathBuf,
}

impl FileObjectStorage {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    async fn directory(&self, key: &ObjectKey) -> Result<PathBuf> {
        let root = self.root.join(key.namespace());
        tokio::fs::create_dir_all(&root)
            .await
            .context("创建插件对象目录失败")?;
        ensure!(
            !tokio::fs::symlink_metadata(&root)
                .await?
                .file_type()
                .is_symlink(),
            "对象目录不能是符号链接"
        );
        Ok(tokio::fs::canonicalize(root).await?)
    }
}

#[async_trait::async_trait]
impl ObjectStorage for FileObjectStorage {
    async fn read(&self, key: &ObjectKey) -> Result<Vec<u8>> {
        let path = self.directory(key).await?.join(key.digest());
        let metadata = tokio::fs::symlink_metadata(&path)
            .await
            .context("对象不存在")?;
        ensure!(
            metadata.is_file() && metadata.len() <= MAX_OBJECT_BYTES as u64,
            "对象类型或大小无效"
        );
        // 即使读入期间文件增长，也不把无限数据载入宿主内存。
        let file = tokio::fs::File::open(path).await?;
        let mut bytes = Vec::new();
        file.take((MAX_OBJECT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .await?;
        ensure!(bytes.len() <= MAX_OBJECT_BYTES, "对象超过 16 MiB 配额");
        Ok(bytes)
    }

    async fn write(&self, key: &ObjectKey, bytes: &[u8]) -> Result<()> {
        let root = self.directory(key).await?;
        let path = root.join(key.digest());
        // RAII 临时文件在超时或任务取消时同样回收；写入只针对已打开的文件句柄。
        let temporary = tokio::task::spawn_blocking(move || tempfile::NamedTempFile::new_in(root))
            .await
            .context("创建对象临时文件任务失败")??;
        let mut file = tokio::fs::File::from_std(temporary.as_file().try_clone()?);
        file.write_all(bytes).await?;
        file.flush().await?;
        tokio::fs::rename(temporary.path(), path).await?;
        Ok(())
    }

    async fn remove(&self, key: &ObjectKey) -> Result<()> {
        let path = self.directory(key).await?.join(key.digest());
        match tokio::fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("删除对象失败"),
        }
    }
}
