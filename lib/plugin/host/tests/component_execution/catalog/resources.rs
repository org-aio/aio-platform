//! 无数据库能力的测试组合，任何误用数据库或发布事务都会明确失败。
use anyhow::Result;
use az_plugin_bundle::Bundle;
use az_plugin_contract::CapabilityGrants;
use az_plugin_core::{Plugin, PluginFuture};
use az_plugin_host::{configuration::ComponentStorage, runtime::server::plugins::*};
use az_plugin_runtime::{
    ComponentDatabase, ComponentReleases, Keyring, ReleaseLease, ScopedDatabase, StoredRelease,
};
use std::sync::Arc;
use uuid::Uuid;

pub struct NoDatabase;
impl Plugin<RuntimeAssembly> for NoDatabase {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_value(Self)
                .bind::<dyn ComponentDatabaseProvider, Self>();
            Ok(())
        })
    }
}
#[async_trait::async_trait]
impl ComponentDatabaseProvider for NoDatabase {
    async fn open(&self, _: &ComponentStorage) -> Result<Arc<dyn ComponentDatabase>> {
        Ok(Arc::new(Self))
    }
}
#[async_trait::async_trait]
impl ComponentDatabase for NoDatabase {
    async fn install(
        &self,
        _: &str,
        _: &str,
        _: &[(String, String)],
        _: &Keyring,
    ) -> Result<ScopedDatabase> {
        anyhow::bail!("测试禁止数据库能力")
    }
    fn releases(&self) -> Arc<dyn ComponentReleases> {
        Arc::new(Self)
    }
    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}
#[async_trait::async_trait]
impl ComponentReleases for NoDatabase {
    async fn initialize(&self) -> Result<()> {
        anyhow::bail!("测试禁止数据库发布记录")
    }
    async fn generation(&self, _: Uuid, _: &str) -> Result<i64> {
        anyhow::bail!("测试禁止数据库发布记录")
    }
    async fn stored(&self, _: Uuid, _: &str) -> Result<Option<StoredRelease>> {
        anyhow::bail!("测试禁止数据库发布记录")
    }
    async fn lease(&self, _: Uuid, _: &str) -> Result<Box<dyn ReleaseLease>> {
        anyhow::bail!("测试禁止数据库发布记录")
    }
    async fn activate(
        &self,
        _: Uuid,
        _: &str,
        _: i64,
        _: &Bundle,
        _: &CapabilityGrants,
    ) -> Result<()> {
        anyhow::bail!("测试禁止数据库发布记录")
    }
    async fn deactivate(&self, _: Uuid, _: &str) -> Result<()> {
        anyhow::bail!("测试禁止数据库发布记录")
    }
}
