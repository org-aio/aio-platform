use std::{any::Any, sync::Arc};

use crate::{ComponentReleases, Keyring, ScopedDatabase};
use anyhow::Result;

/// 组件数据库实现；原生插件负责租户隔离、受控迁移及持久发布后端的生命周期。
#[async_trait::async_trait]
pub trait ComponentDatabase: Any + Send + Sync {
    /// 准备租户数据后返回专属连接，不能暴露平台管理凭据或未隔离的数据。
    async fn install(
        &self,
        source: &str,
        tenant: &str,
        migrations: &[(String, String)],
        keyring: &Keyring,
    ) -> Result<ScopedDatabase>;

    /// 宿主发布记录，与租户业务事务隔离，只交给可信执行器。
    fn releases(&self) -> Arc<dyn ComponentReleases>;

    /// 执行器全部退出后关闭本次资源，保留租户数据和安装历史。
    async fn shutdown(&self) -> Result<()>;
}
