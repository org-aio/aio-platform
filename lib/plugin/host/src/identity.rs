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

/// 权限管理的受控读写；只有真实权限提供者能实现，开发替身默认拒绝。
///
/// 每个方法都由提供者在**同一个事务**里复核调用者仍持有 `rbac:manage`，
/// 并拒绝超出调用者自身授权范围的分配；宿主只负责门控与会话绑定。
#[cfg(feature = "server")]
#[async_trait::async_trait]
pub trait AccessControlProvider: std::any::Any + Send + Sync {
    /// 读取当前租户的用户、角色、部门、岗位与可授予权限。
    async fn access_control_view(
        &self,
        _session: &SessionContext,
    ) -> anyhow::Result<AccessControlView> {
        anyhow::bail!("当前权限提供者不支持读取权限视图")
    }

    /// 创建或更新角色；`create` 为真时要求角色不存在。
    async fn save_role(
        &self,
        _session: &SessionContext,
        _role_id: &str,
        _permissions: &[String],
        _create: bool,
    ) -> anyhow::Result<()> {
        anyhow::bail!("当前权限提供者不支持保存角色")
    }

    /// 删除角色；仍被分配的角色必须被拒绝。
    async fn delete_role(&self, _session: &SessionContext, _role_id: &str) -> anyhow::Result<()> {
        anyhow::bail!("当前权限提供者不支持删除角色")
    }

    /// 更新成员显示名称。
    async fn update_member(
        &self,
        _session: &SessionContext,
        _user_id: &str,
        _display_name: &str,
    ) -> anyhow::Result<()> {
        anyhow::bail!("当前权限提供者不支持更新成员")
    }

    /// 覆盖成员的角色分配。
    async fn set_member_roles(
        &self,
        _session: &SessionContext,
        _user_id: &str,
        _roles: &[String],
    ) -> anyhow::Result<()> {
        anyhow::bail!("当前权限提供者不支持分配角色")
    }

    /// 把成员移出当前租户。
    async fn remove_member(&self, _session: &SessionContext, _user_id: &str) -> anyhow::Result<()> {
        anyhow::bail!("当前权限提供者不支持移除成员")
    }

    /// 只做权限判定，不写密码；实际写入由身份服务完成。
    async fn authorize_member_password_reset(
        &self,
        _session: &SessionContext,
        _user_id: &str,
    ) -> anyhow::Result<()> {
        anyhow::bail!("当前权限提供者不支持重置成员密码")
    }

    /// 写入或更新部门。
    async fn save_department(
        &self,
        _session: &SessionContext,
        _department: AccessControlDepartment,
    ) -> anyhow::Result<()> {
        anyhow::bail!("当前权限提供者不支持保存部门")
    }

    /// 删除部门；存在下级部门时必须拒绝。
    async fn delete_department(&self, _session: &SessionContext, _id: &str) -> anyhow::Result<()> {
        anyhow::bail!("当前权限提供者不支持删除部门")
    }

    /// 写入或更新岗位。
    async fn save_post(
        &self,
        _session: &SessionContext,
        _post: AccessControlPost,
    ) -> anyhow::Result<()> {
        anyhow::bail!("当前权限提供者不支持保存岗位")
    }

    /// 删除岗位。
    async fn delete_post(&self, _session: &SessionContext, _id: &str) -> anyhow::Result<()> {
        anyhow::bail!("当前权限提供者不支持删除岗位")
    }
}

/// 权限视图；宿主与插件共用，避免插件自行定义第二套形状。
#[cfg(feature = "server")]
pub use az_plugin_contract::process::{
    AccessControlDepartment, AccessControlOrganization, AccessControlPost, AccessControlRole,
    AccessControlUser, AccessControlView,
};

/// 租户视图；宿主与插件共用，避免插件自行定义第二套形状。
#[cfg(feature = "server")]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct TenantView {
    pub id: String,
    pub label: String,
    pub current: bool,
}
