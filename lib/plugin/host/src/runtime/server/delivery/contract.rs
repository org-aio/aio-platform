//! 交付协议的中立边界；具体提供者负责持久状态、租约和发布的一致性。
use anyhow::Result;
use az_plugin_delivery::{BuildJob, BuildReport, DocumentImage, Documentation};
use serde_json::Value;
use std::any::Any;

/// 上传格式由协议入口识别，提供者必须校验包的来源、提交、版本及有效租约。
#[derive(Clone, Copy, Debug)]
pub enum DeliveryArtifact {
    Plugin,
    Component,
}

/// 构建队列与发布协调服务。所有写操作在提交时复核租约和当前目标；
/// 失败必须可恢复，旧任务不能覆盖新任务，并保留租户停用、卸载和回滚选择。
/// HTTP 入口只做认证及协议转换；后台发现和恢复由提供者插件的任务作用域拥有。
#[async_trait::async_trait]
pub trait DeliveryService: Any + Send + Sync {
    async fn claim(&self) -> Result<Option<BuildJob>>;
    async fn retained_jobs(&self) -> Result<Vec<i64>>;
    async fn defer(&self, id: i64, lease: &str, error: &str) -> Result<bool>;
    async fn heartbeat(&self, id: i64, lease: &str) -> Result<bool>;
    async fn upload(
        &self,
        id: i64,
        lease: &str,
        format: DeliveryArtifact,
        bytes: &[u8],
    ) -> Result<()>;
    async fn complete(&self, id: i64, report: BuildReport) -> Result<()>;
    async fn retry(&self, id: i64) -> Result<bool>;
    /// 只由通过发布管理权限检查的入口读取，错误和构建信息不向普通会话暴露。
    async fn builds(&self, git: &str) -> Result<Vec<Value>>;
}

/// 已发布旧版包的文档。读取时必须排除未发布的包；同一 revision 的文档不可覆盖。
/// details 返回既有文档协议的对象（readme、version、source_revision、git、versions），
/// 不包含构建历史；公共入口按当前会话权限单独聚合构建历史。
#[async_trait::async_trait]
pub trait PluginDocumentation: Any + Send + Sync {
    async fn save(&self, revision: &str, documentation: &Documentation) -> Result<()>;
    async fn details(&self, revision: &str) -> Result<Option<Value>>;
    async fn image(&self, revision: &str, path: &str) -> Result<Option<DocumentImage>>;
}
