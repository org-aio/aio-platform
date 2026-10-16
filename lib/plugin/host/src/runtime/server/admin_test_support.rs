//! 隔离夹具复用正式插件装配与资源初始化，仅跳过业务恢复和后台任务。
use super::RuntimeState;
use crate as az_plugin_host;
include!("../../../tests/support/identity.rs");
use anyhow::Result;
use std::{path::Path, sync::Arc};

impl RuntimeState {
    pub async fn isolated_admin_test<
        P: crate::identity::IdentityProvider + crate::authorization::AuthorizationProvider,
    >(
        identity: Arc<P>,
        database: &str,
        origin: &str,
        cache: &Path,
    ) -> Result<Self> {
        Self::isolated_test(identity, database, origin, cache, false).await
    }

    #[cfg(test)]
    pub(super) async fn isolated_component_test<
        P: crate::identity::IdentityProvider + crate::authorization::AuthorizationProvider,
    >(
        identity: Arc<P>,
        database: &str,
        origin: &str,
        cache: &Path,
    ) -> Result<Self> {
        Self::isolated_test(identity, database, origin, cache, true).await
    }

    async fn isolated_test<
        P: crate::identity::IdentityProvider + crate::authorization::AuthorizationProvider,
    >(
        identity: Arc<P>,
        database: &str,
        origin: &str,
        cache: &Path,
        components: bool,
    ) -> Result<Self> {
        let config = crate::configuration::HostConfig {
            remote_capabilities: None,
            database_url: database.into(),
            cache_root: cache.into(),
            public_origin: origin.into(),
            component_storage: components.then(|| crate::configuration::ComponentStorage {
                database_url: database.into(),
                root: cache.into(),
            }),
            default_plugins: vec![],
            delivery: None,
            development: None,
        };
        Self::initialize_selected(
            config,
            with_identity(identity, super::plugins::default_plugins()?),
            false,
        )
        .await
    }
}
