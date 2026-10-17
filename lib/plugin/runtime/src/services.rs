use anyhow::Result;
use async_trait::async_trait;
use az_plugin_contract::{EnhancementRequest, EnhancementResponse, InvocationScope};

use az_plugin_contract::transport::{Request, Response};

#[async_trait]
pub trait HostServices: Send + Sync {
    async fn authorize(&self, scope: &InvocationScope, permission: &str) -> Result<bool>;
    /// 管理入口独立核验部署信任、活动版本、真实交互会话及宿主权限；不能复用组件命名空间授权。
    async fn manage(&self, scope: &InvocationScope, request: Request) -> Result<Response>;

    /// 执行宿主提供的版本化纯计算能力；未绑定增强的宿主明确拒绝调用。
    async fn enhance(
        &self,
        _: &InvocationScope,
        _: EnhancementRequest,
    ) -> Result<EnhancementResponse> {
        anyhow::bail!("宿主尚未绑定脚本增强服务")
    }
}
