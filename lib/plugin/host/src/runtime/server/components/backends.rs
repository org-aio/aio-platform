use std::{panic::AssertUnwindSafe, sync::Arc};

use anyhow::{Result, ensure};
use az_plugin_runtime::{ComponentDatabase, ObjectStorage};
use futures_util::FutureExt;

use super::super::plugins::{ComponentDatabaseProvider, ObjectStorageProvider};
use crate::configuration::ComponentStorage;

pub(super) struct ComponentBackends {
    pub database: Arc<dyn ComponentDatabase>,
    pub storage: Arc<dyn ObjectStorage>,
}

impl ComponentBackends {
    pub async fn open(catalog: &dill::Catalog, config: &ComponentStorage) -> Result<Self> {
        // 先解析全部唯一绑定，避免装配错误发生在资源创建之后。
        let database_provider = catalog.get_one::<dyn ComponentDatabaseProvider>()?;
        let storage_provider = catalog.get_one::<dyn ObjectStorageProvider>()?;
        let database = database_provider.open(config).await?;
        let storage = AssertUnwindSafe(async { storage_provider.open(config).await })
            .catch_unwind()
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("对象存储初始化时 panic")));
        match storage {
            Ok(storage) => Ok(Self { database, storage }),
            Err(error) => match close_database(database.as_ref()).await {
                Ok(()) => Err(error),
                Err(cleanup) => Err(error.context(format!("数据库补偿失败: {cleanup:#}"))),
            },
        }
    }

    pub async fn shutdown(&self) -> Result<()> {
        let storage = AssertUnwindSafe(async { self.storage.shutdown().await })
            .catch_unwind()
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("对象存储关闭时 panic")));
        // 对象后端失败或 panic 也必须继续释放数据库。
        let database = close_database(self.database.as_ref()).await;
        let errors: Vec<_> = [storage, database]
            .into_iter()
            .filter_map(Result::err)
            .map(|error| format!("{error:#}"))
            .collect();
        ensure!(errors.is_empty(), "关闭组件资源失败: {}", errors.join("; "));
        Ok(())
    }
}

async fn close_database(database: &dyn ComponentDatabase) -> Result<()> {
    AssertUnwindSafe(async { database.shutdown().await })
        .catch_unwind()
        .await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("组件数据库关闭时 panic")))
}
