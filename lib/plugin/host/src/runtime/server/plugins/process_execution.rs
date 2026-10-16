//! v1 进程执行契约；监督器和客户端实现可独立编译、替换。

use std::any::Any;

use anyhow::Result;
use az_plugin_manifest::PageDefinition;
use serde::{Deserialize, Serialize};

/// 监督器启动请求，实例身份由租户、来源及版本共同决定。
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StartProcessRequest {
    pub tenant_id: String,
    pub source_id: String,
    pub revision: String,
}

#[cfg(feature = "sidecar")]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct StopProcessRequest {
    pub instance_id: String,
}

#[cfg(feature = "sidecar")]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReconcileProcessesRequest {
    pub instances: Vec<StartProcessRequest>,
}

/// 监督器返回的进程归属；仅本次 created 的实例由本次宿主清理。
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProcessInstance {
    pub instance_id: String,
    pub endpoint: String,
    pub created: bool,
}

/// v1 服务代理响应，由宿主路由转成 HTTP。
pub struct ProcessResponse {
    pub status: reqwest::StatusCode,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

/// 进程执行服务，调用者通过宿主生命周期和权限检查后使用。
#[async_trait::async_trait]
pub trait ProcessExecutor: Any + Send + Sync {
    async fn health(&self) -> Result<()>;
    async fn start(
        &self,
        tenant_id: &str,
        source_id: &str,
        revision: &str,
    ) -> Result<ProcessInstance>;
    async fn stop(&self, instance_id: &str) -> Result<()>;
    async fn reconcile(&self, instances: Vec<StartProcessRequest>) -> Result<()>;
    async fn load_pages(&self, endpoint: &str) -> Result<Vec<PageDefinition>>;
    #[allow(clippy::too_many_arguments)]
    async fn request(
        &self,
        endpoint: &str,
        method: &str,
        path: &str,
        query: Option<&str>,
        body: Vec<u8>,
        content_type: Option<&str>,
        tenant_id: &str,
        user_id: &str,
    ) -> Result<ProcessResponse>;
    async fn shutdown(&self) -> Result<()>;
}
