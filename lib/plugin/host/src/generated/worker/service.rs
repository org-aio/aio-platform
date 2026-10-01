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
    async fn task(&self, session: &SessionContext, id: &str) -> Result<Task>;
    /// 登录用户取消本人任务，已结束任务保持原结果并幂等返回。
    async fn cancel_task(&self, session: &SessionContext, id: &str) -> Result<Task>;
    /// 本机设备凭据只管理自己的工作区执行能力。
    async fn workspace_access(&self, device: &DeviceIdentity, enabled: bool) -> Result<()>;
    async fn desktop_access(&self, device: &DeviceIdentity, enabled: bool) -> Result<()>;
    async fn desktop(&self, session: &SessionContext, id: &str, enabled: bool) -> Result<()>;
    /// 浏览器创建的终端会话只能由同一账号的在线设备领取。
    async fn terminal_create(
        &self,
        session: &SessionContext,
        request: CreateTerminal,
    ) -> Result<TerminalSession>;
    /// 浏览器只能选择同一账号中已启用 terminal.open 的在线设备。
    async fn terminal_devices(&self, session: &SessionContext) -> Result<Vec<Worker>>;
    async fn terminal_events(
        &self,
        session: &SessionContext,
        id: &str,
        after: u64,
        wait_seconds: u8,
    ) -> Result<TerminalEvents>;
    async fn terminal_input(
        &self,
        session: &SessionContext,
        id: &str,
        request: TerminalInput,
    ) -> Result<TerminalSession>;
    async fn terminal_resize(
        &self,
        session: &SessionContext,
        id: &str,
        request: TerminalResize,
    ) -> Result<TerminalSession>;
    async fn terminal_close(&self, session: &SessionContext, id: &str) -> Result<()>;
    async fn terminal_claim(
        &self,
        device: &DeviceIdentity,
        wait_seconds: u8,
    ) -> Result<Option<TerminalSession>>;
    async fn terminal_read(
        &self,
        device: &DeviceIdentity,
        id: &str,
        after: u64,
        wait_seconds: u8,
    ) -> Result<TerminalEvents>;
    async fn terminal_write(
        &self,
        device: &DeviceIdentity,
        id: &str,
        request: TerminalInput,
    ) -> Result<()>;
    async fn terminal_finish(
        &self,
        device: &DeviceIdentity,
        id: &str,
        request: TerminalFinish,
    ) -> Result<()>;
    async fn terminal_access(&self, device: &DeviceIdentity, enabled: bool) -> Result<()>;
    async fn claim(&self, device: &DeviceIdentity, request_id: &str) -> Result<Option<Task>>;
    async fn heartbeat(&self, device: &DeviceIdentity, id: Option<(&str, &str)>) -> Result<()>;
    async fn complete(
        &self,
        device: &DeviceIdentity,
        id: &str,
        request: CompleteTask,
    ) -> Result<()>;
}
