use std::{any::Any, path::PathBuf};

use anyhow::Result;

use crate::{DatabaseRows, DatabaseValue};

/// 数据库提供者返回的专属租户连接。实现负责数据库层隔离和语句策略，
/// 不能允许业务 SQL 改变事务边界、迁移结构、权限或访问其他租户。
#[async_trait::async_trait]
pub trait DatabaseBackend: Any + Send + Sync {
    /// 只验证迁移，不能在执行槽准备阶段重复执行迁移；实际应用由 Provider 完成。
    fn validate_migrations(&self, migrations: &[(String, String)]) -> Result<()>;
    async fn begin(&self) -> Result<Box<dyn DatabaseTransaction>>;

    /// Sidecar 只能获得同一专属连接的凭据；不支持网络直连的后端明确拒绝。
    fn process_connection(&self) -> Result<ProcessDatabaseConnection> {
        anyhow::bail!("数据库后端不支持 Sidecar 的隔离网络连接")
    }
}

/// 后端事务由运行时独占。取消请求或丢弃句柄必须回滚，禁止在 Drop 中提交。
/// 驱动还应在读取过程中限制结果大小，避免一次分配不受限的数据。
#[async_trait::async_trait]
pub trait DatabaseTransaction: Any + Send + Sync {
    async fn query(
        &mut self,
        statement: &str,
        parameters: Vec<DatabaseValue>,
    ) -> Result<DatabaseRows>;
    async fn execute(&mut self, statement: &str, parameters: Vec<DatabaseValue>) -> Result<u64>;
    async fn commit(self: Box<Self>) -> Result<()>;
    async fn rollback(self: Box<Self>) -> Result<()>;
}

/// 私有 Sidecar 网络代理的上游地址，不充当插件身份。
pub enum DatabaseEndpoint {
    Tcp { host: String, port: u16 },
    Unix(PathBuf),
}

/// 仅交给可信宿主的连接信息；包含凭据，因此不实现 Debug 或序列化。
/// 隔离进程通过 /database 下的单个 socket 访问上游，不能任意联网。
pub struct ProcessDatabaseConnection {
    pub direct_url: String,
    pub isolated_url: String,
    pub socket_name: String,
    pub upstream: DatabaseEndpoint,
}
