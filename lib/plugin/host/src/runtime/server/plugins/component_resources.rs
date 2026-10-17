use std::{any::Any, sync::Arc};

use anyhow::Result;
use az_plugin_core::{Plugin, PluginFuture};
use az_plugin_runtime::{ComponentDatabase, FileObjectStorage, ObjectStorage};

use super::RuntimeAssembly;
use crate::configuration::ComponentStorage;
#[cfg(feature = "postgres")]
use az_plugin_runtime::DatabaseProvisioner;

/// 创建当前组件宿主拥有的数据库后端；打开失败时由实现清理尚未返回的资源。
#[async_trait::async_trait]
pub trait ComponentDatabaseProvider: Any + Send + Sync {
    async fn open(&self, config: &ComponentStorage) -> Result<Arc<dyn ComponentDatabase>>;
}

/// 创建当前组件宿主拥有的对象后端；宿主负责执行器退出后的关闭及失败补偿。
#[async_trait::async_trait]
pub trait ObjectStorageProvider: Any + Send + Sync {
    async fn open(&self, config: &ComponentStorage) -> Result<Arc<dyn ObjectStorage>>;
}

/// 默认受控 PostgreSQL 迁移实现；应用可省略此插件并绑定替代 Provider。
#[cfg(feature = "postgres")]
#[dill::component]
#[dill::interface(dyn ComponentDatabaseProvider)]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct PostgresComponentPlugin;

#[cfg(feature = "postgres")]
#[async_trait::async_trait]
impl ComponentDatabaseProvider for PostgresComponentPlugin {
    async fn open(&self, config: &ComponentStorage) -> Result<Arc<dyn ComponentDatabase>> {
        let database = DatabaseProvisioner::connect(&config.database_url).await?;
        Ok(Arc::new(database))
    }
}

#[cfg(feature = "postgres")]
impl Plugin<RuntimeAssembly> for PostgresComponentPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.services.add::<Self>();
            Ok(())
        })
    }
}

/// 默认文件对象实现；路径只由宿主配置决定，租户命名空间由运行时生成。
#[dill::component]
#[dill::interface(dyn ObjectStorageProvider)]
#[dill::interface(dyn Plugin<RuntimeAssembly>)]
pub struct FileObjectStoragePlugin;

#[async_trait::async_trait]
impl ObjectStorageProvider for FileObjectStoragePlugin {
    async fn open(&self, config: &ComponentStorage) -> Result<Arc<dyn ObjectStorage>> {
        Ok(Arc::new(FileObjectStorage::new(
            config.root.join("objects"),
        )))
    }
}

impl Plugin<RuntimeAssembly> for FileObjectStoragePlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.services.add::<Self>();
            Ok(())
        })
    }
}
