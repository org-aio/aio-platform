use super::model::*;
use crate::runtime::server::http_error::RuntimeError;

/// 网页和已授权设备共用剪切板通道，所有读写均限定所有者。
#[async_trait::async_trait]
pub(crate) trait ClipboardService: Send + Sync {
    /// 当前槽位与最新条目，用于界面和变更轮询。
    async fn head(&self, owner: &Owner) -> Result<ClipHead, RuntimeError>;
    /// 按序号倒序分页；`cursor` 为空时从最新开始。
    async fn list(
        &self,
        owner: &Owner,
        cursor: Option<i64>,
        limit: i64,
    ) -> Result<ClipPage, RuntimeError>;
    /// 读取指定条目的原始字节（Base64）。
    async fn read(&self, owner: &Owner, id: &str) -> Result<ClipContent, RuntimeError>;
    /// 追加新条目并推进槽位版本，返回其元数据。
    async fn write(&self, owner: &Owner, request: ClipWrite) -> Result<ClipItem, RuntimeError>;
    /// 列出已配对的设备及其剪切板通道开关。
    async fn devices(&self, owner: &Owner) -> Result<Vec<ClipboardDevice>, RuntimeError>;
    /// 仅本机设备可以开通或关闭自己的剪切板通道。
    async fn access(&self, owner: &Owner, device: &str, enabled: bool) -> Result<(), RuntimeError>;
}
