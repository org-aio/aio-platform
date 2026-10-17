use std::any::Any;

use anyhow::Result;
use az_plugin_bundle::Bundle;
use az_plugin_contract::CapabilityGrants;
use uuid::Uuid;

/// 发布记录中的版本和代数；停用仍保留代数，防止旧候选重新激活。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseVersion {
    pub revision: Option<String>,
    pub generation: i64,
}

/// 已提交的完整发布；整包摘要必须由存储实现核验。
pub struct StoredRelease {
    pub generation: i64,
    pub bundle: Bundle,
    pub grants: CapabilityGrants,
}

/// 调用期间持有的版本读租约；其存活期间，其他宿主的发布与停用必须等待。
/// 取消或丢弃租约必须释放锁，finish 负责暴露后端收尾错误。
#[async_trait::async_trait]
pub trait ReleaseLease: Any + Send + Sync {
    fn version(&self) -> &ReleaseVersion;
    async fn finish(self: Box<Self>) -> Result<()>;
}

/// 整包发布的持久化边界，不暴露数据库连接或 SQL 事务。
/// 所有操作按来源和租户隔离；激活必须原子比较代数并提交整包、授权和审计。
#[async_trait::async_trait]
pub trait ComponentReleases: Any + Send + Sync {
    async fn initialize(&self) -> Result<()>;
    async fn generation(&self, source: Uuid, tenant: &str) -> Result<i64>;
    async fn stored(&self, source: Uuid, tenant: &str) -> Result<Option<StoredRelease>>;
    async fn lease(&self, source: Uuid, tenant: &str) -> Result<Box<dyn ReleaseLease>>;
    async fn activate(
        &self,
        source: Uuid,
        tenant: &str,
        expected: i64,
        bundle: &Bundle,
        grants: &CapabilityGrants,
    ) -> Result<()>;
    async fn deactivate(&self, source: Uuid, tenant: &str) -> Result<()>;
}
