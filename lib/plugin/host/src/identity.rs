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

    /// 代表进程插件上报用量。默认不支持，宿主按需覆盖。
    ///
    /// 返回值 `Ok(None)` 表示该宿主未接入计费，调用方不应视为失败。
    async fn meter(
        &self,
        _tenant: &str,
        _user: &str,
        _source: &str,
        _resource: &str,
        _quantity: i64,
        _idempotency_key: &str,
    ) -> anyhow::Result<Option<MeterOutcome>> {
        Ok(None)
    }
}

/// 一次用量上报的结算结果。
#[cfg(feature = "server")]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeterOutcome {
    pub amount_micros: i64,
    pub grant_consumed: i64,
    pub balance_charged_micros: i64,
    pub balance_after_micros: i64,
    pub duplicate: bool,
}
