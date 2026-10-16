use std::sync::Arc;

use crate::{HostServices, Keyring, ObjectStore, ScopedDatabase};

/// 宿主按租户和能力授予执行器的资源；不持有具体语言引擎。
#[derive(Clone, Default)]
pub struct InvocationResources {
    pub keyring: Option<Arc<Keyring>>,
    pub database: Option<ScopedDatabase>,
    pub storage: Option<ObjectStore>,
    pub services: Option<Arc<dyn HostServices>>,
}

impl InvocationResources {
    /// 执行器使用资源前校验来源和租户，防止实例误用其他调用的存储绑定。
    pub fn validate_scope(&self, source: &str, tenant: &str) -> anyhow::Result<()> {
        if let Some(storage) = &self.storage {
            anyhow::ensure!(
                storage.matches(source, tenant),
                "对象存储绑定不属于当前插件和租户"
            );
        }
        if let Some(database) = &self.database {
            anyhow::ensure!(
                database.matches(source, tenant),
                "数据库绑定不属于当前插件和租户"
            );
        }
        Ok(())
    }
}
