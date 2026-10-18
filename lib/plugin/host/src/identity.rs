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
pub trait IdentityProvider: std::any::Any + Send + Sync {
    async fn authenticate(
        &self,
        headers: &axum::http::HeaderMap,
    ) -> anyhow::Result<Option<SessionContext>>;

    async fn member_active(&self, tenant: &str, user: &str) -> anyhow::Result<bool>;

    async fn session_active(&self, session: &str, tenant: &str, user: &str)
    -> anyhow::Result<bool>;

    /// 在指定租户下创建账号，返回新用户 ID。
    ///
    /// 只有真实身份提供者能实现。默认拒绝，避免开发替身在产品路径上放行创建。
    async fn create_user(
        &self,
        _tenant: &str,
        _account: &str,
        _display_name: &str,
        _password: &str,
    ) -> anyhow::Result<String> {
        anyhow::bail!("当前身份提供者不支持创建账号")
    }

    /// 重置指定用户密码；提供者负责校验目标存在与密码策略。
    async fn reset_password(&self, _user_id: &str, _new_password: &str) -> anyhow::Result<()> {
        anyhow::bail!("当前身份提供者不支持重置密码")
    }

    /// 把会话切换到目标租户，并返回切换后的会话；成员关系由提供者校验。
    async fn switch_tenant(
        &self,
        _session: &SessionContext,
        _tenant_id: &str,
    ) -> anyhow::Result<Option<SessionContext>> {
        anyhow::bail!("当前身份提供者不支持切换租户")
    }

    /// 列出该用户加入的租户，并标记会话当前所在租户。
    async fn list_tenants(&self, _session: &SessionContext) -> anyhow::Result<Vec<TenantView>> {
        anyhow::bail!("当前身份提供者不支持租户列表")
    }

    /// 创建租户并把调用者设为管理员，返回新租户。
    async fn create_tenant(
        &self,
        _session: &SessionContext,
        _label: &str,
    ) -> anyhow::Result<TenantView> {
        anyhow::bail!("当前身份提供者不支持创建租户")
    }

    /// 重命名会话当前所在租户。
    async fn rename_current_tenant(
        &self,
        _session: &SessionContext,
        _label: &str,
    ) -> anyhow::Result<TenantView> {
        anyhow::bail!("当前身份提供者不支持重命名租户")
    }
}

/// 租户视图；宿主与插件共用，避免插件自行定义第二套形状。
#[cfg(feature = "server")]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct TenantView {
    pub id: String,
    pub label: String,
    pub current: bool,
}
