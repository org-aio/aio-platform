//! 系统整包的访问门槛由当前安装版本声明；权限只从宿主身份和授权插件查询。

use anyhow::{Context, Result, ensure};
use uuid::Uuid;

use super::Components;
use crate::{authorization::AuthorizationProvider, identity::IdentityProvider};

impl Components {
    pub(super) async fn required_permissions(
        &self,
        tenant: &str,
        source: Uuid,
        digest: &str,
    ) -> Result<Vec<String>> {
        if tenant == "development"
            && let Some(instance) = self.development.read().await.get(&source)
        {
            ensure!(instance.bundle.digest() == digest, "插件活动版本已变化");
            return Ok(instance
                .bundle
                .manifest()
                .plugin
                .required_permissions
                .clone());
        }
        sqlx::query_scalar(
            "SELECT v.required_permissions FROM component_installations i JOIN component_versions v ON v.digest=i.digest WHERE i.source_id=$1 AND i.tenant_id=$2 AND i.digest=$3 AND i.enabled",
        )
        .bind(source)
        .bind(tenant)
        .bind(digest)
        .fetch_optional(&self.pool)
        .await?
        .context("插件活动版本已变化或未启用")
    }

    // 空要求保持普通业务插件的访问语义；高权限包必须有真实且仍有效的交互会话。
    pub(super) async fn host_permissions(
        &self,
        tenant: &str,
        user: &str,
        session: &str,
        required: &[String],
    ) -> Result<bool> {
        if required.is_empty() {
            return Ok(true);
        }
        if tenant.is_empty()
            || user.is_empty()
            || session.is_empty()
            || session.starts_with("service:")
        {
            return Ok(false);
        }
        let services = self.runtime_services.upgrade();
        let identity = services.get_one::<dyn IdentityProvider>()?;
        if !identity.member_active(tenant, user).await?
            || !identity.session_active(session, tenant, user).await?
        {
            return Ok(false);
        }
        services
            .get_one::<dyn AuthorizationProvider>()?
            .session_has_permissions(session, tenant, user, required)
            .await
    }
}
