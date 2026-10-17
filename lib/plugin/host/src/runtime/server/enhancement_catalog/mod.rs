//! 增强持久化契约；语言运行时、演进策略及所有传输共享同一个提供者。

mod model;
mod observations;
mod postgres;
mod postgres_rows;

use std::{any::Any, future::Future, pin::Pin};

use anyhow::Result;
use az_plugin_enhancement::Package;

pub use model::{Activation, Event, Head, Observation, Revision};
pub use postgres::PostgresEnhancementCatalog;

/// 宿主创建的提交前复核；存储取得写锁并完成校验后必须等待它，再提交事务。
/// 拒绝、丢弃或取消时必须回滚，不能提前执行后缓存授权结果。
pub type CommitAuthorization<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

/// 提供者负责租户隔离、不可变版本与原子写入；宿主负责脚本编译、样例及实时授权。
/// 所有错误必须传播；不得回退到空目录或在失败后留下活动指针、事件或候选。
#[async_trait::async_trait]
pub trait EnhancementCatalog: Any + Send + Sync {
    async fn head(&self, tenant: &str) -> Result<Head>;
    /// 最新 50 个候选，按创建时间倒序、摘要排序。
    async fn revisions(&self, tenant: &str) -> Result<Vec<Revision>>;
    async fn package(&self, tenant: &str, revision: &str) -> Result<Package>;
    /// 摘要由包内容决定；重复提交不得覆盖原候选及其作者。
    async fn propose(
        &self,
        tenant: &str,
        author: &str,
        package: &Package,
        authorize: CommitAuthorization<'_>,
    ) -> Result<()>;
    /// CAS、历史 schema 约束、回滚资格和事件同事务提交。代数冲突返回 None。
    /// 停用和回滚不能释放曾发布路径的 schema；回滚仅限本租户曾激活的版本。
    async fn activate(
        &self,
        tenant: &str,
        actor: &str,
        request: &Activation,
        rollback: bool,
        authorize: CommitAuthorization<'_>,
    ) -> Result<Option<Head>>;
    /// 最新 50 个事件，按激活代数倒序。
    async fn history(&self, tenant: &str) -> Result<Vec<Event>>;
    /// 仅允许查询本租户已激活的版本与代数，结果按路径排序。
    async fn observe(
        &self,
        tenant: &str,
        revision: &str,
        generation: i64,
    ) -> Result<Vec<Observation>>;
    /// 调用次数按激活代数计数；不保存输入、输出或凭据。
    async fn started(
        &self,
        tenant: &str,
        revision: &str,
        generation: i64,
        path: &str,
    ) -> Result<()>;
    async fn finished(
        &self,
        tenant: &str,
        generation: i64,
        path: &str,
        success: bool,
    ) -> Result<()>;
}
