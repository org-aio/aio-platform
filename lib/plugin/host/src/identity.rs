use serde::{Deserialize, Serialize};

/// 运行时只消费已验证的身份与权限，不感知登录、账号存储或产品账户页面。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionContext {
    pub session_id: String,
    pub user_id: String,
    pub account: String,
    pub display_name: String,
    pub tenant_id: String,
    pub tenant_label: String,
    pub permissions: Vec<String>,
}

#[cfg(feature = "server")]
#[async_trait::async_trait]
pub trait IdentityProvider: Send + Sync {
    async fn authenticate(
        &self,
        headers: &axum::http::HeaderMap,
    ) -> anyhow::Result<Option<SessionContext>>;

    async fn can_publish(&self, session: &SessionContext) -> anyhow::Result<bool>;

    async fn member_active(&self, tenant: &str, user: &str) -> anyhow::Result<bool>;

    async fn session_active(&self, session: &str, tenant: &str, user: &str)
    -> anyhow::Result<bool>;
}
