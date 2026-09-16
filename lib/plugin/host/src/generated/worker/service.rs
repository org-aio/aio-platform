use super::model::*;
use crate::identity::SessionContext;
use anyhow::Result;
use std::any::Any;

/// 宿主注册的设备和任务服务，不处理登录密码或实际执行任务。
#[async_trait::async_trait]
pub(crate) trait WorkerService: Any + Send + Sync {
    async fn pair(&self, request: PairRequest) -> Result<Pairing>;
    async fn pairing(&self, code: &str) -> Result<Worker>;
    async fn approve(&self, session: &SessionContext, code: &str) -> Result<()>;
    async fn poll(&self, token: &str) -> Result<String>;
    async fn identity(&self, token: &str) -> Result<DeviceIdentity>;
    async fn list(&self, session: &SessionContext) -> Result<Vec<Worker>>;
    async fn revoke(&self, session: &SessionContext, id: &str) -> Result<()>;
    async fn enqueue(&self, session: &SessionContext, request: SubmitTask) -> Result<Task>;
    async fn tasks(&self, session: &SessionContext) -> Result<Vec<Task>>;
    async fn claim(&self, device: &DeviceIdentity) -> Result<Option<Task>>;
    async fn heartbeat(&self, device: &DeviceIdentity, id: Option<(&str, &str)>) -> Result<()>;
    async fn complete(
        &self,
        device: &DeviceIdentity,
        id: &str,
        request: CompleteTask,
    ) -> Result<()>;
}
