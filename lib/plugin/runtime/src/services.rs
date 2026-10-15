use anyhow::Result;
use async_trait::async_trait;
use az_plugin_contract::{EnhancementRequest, EnhancementResponse, InvocationScope};

use crate::bindings::aio::plugin::transport::{Request, Response};

#[async_trait]
pub trait HostServices: Send + Sync {
    async fn authorize(&self, scope: &InvocationScope, permission: &str) -> Result<bool>;
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
