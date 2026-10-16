//! 发布策略与实时授权查询由产品插件提供，身份认证不持有这些业务规则。

use std::any::Any;

use crate::identity::SessionContext;

#[async_trait::async_trait]
pub trait AuthorizationProvider: Any + Send + Sync {
    async fn can_publish(&self, session: &SessionContext) -> anyhow::Result<bool>;

    /// 高权限能力提交前查询当前角色；未实现实时查询时拒绝提交。
    /// 调用者身份由宿主绑定，permissions 是业务权限而非插件身份。
    async fn session_has_permissions(
        &self,
        _session: &str,
        _tenant: &str,
        _user: &str,
        _permissions: &[String],
    ) -> anyhow::Result<bool> {
        Ok(false)
    }
}
