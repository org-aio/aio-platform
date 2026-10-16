use std::any::Any;

use anyhow::Result;
use sqlx::PgPool;

use crate::{Keyring, ScopedDatabase};

/// 组件数据库实现；原生插件可以替换迁移和凭据来源，持久化协议仍为 PostgreSQL。
#[async_trait::async_trait]
pub trait ComponentDatabase: Any + Send + Sync {
    /// 准备租户数据后返回通过统一角色检查的连接，不能返回平台管理角色。
    async fn install(
        &self,
        source: &str,
        tenant: &str,
        migrations: &[(String, String)],
        keyring: &Keyring,
    ) -> Result<ScopedDatabase>;

    /// 宿主的发布记录连接池，只交给可信执行器，不交给脚本或 Sidecar。
    fn release_pool(&self) -> PgPool;

    /// 执行器全部退出后关闭本次资源，保留租户数据和安装历史。
    async fn shutdown(&self) -> Result<()>;
}
