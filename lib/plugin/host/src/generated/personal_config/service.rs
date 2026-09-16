use super::model::*;
use crate::runtime::server::http_error::RuntimeError;
use serde_json::Value;

/// 网页和已授权设备共用个人配置服务，所有读取与写入均限定所有者。
#[async_trait::async_trait]
pub(crate) trait PersonalConfigService: Send + Sync {
    async fn catalog(&self, owner: &Owner) -> Result<Catalog, RuntimeError>;
    async fn revision(&self, owner: &Owner) -> Result<i64, RuntimeError>;
    async fn read(
        &self,
        owner: &Owner,
        id: &str,
        revision: Option<i64>,
    ) -> Result<Content, RuntimeError>;
    async fn write(&self, owner: &Owner, request: WriteEntry) -> Result<Entry, RuntimeError>;
    async fn history(&self, owner: &Owner, id: &str) -> Result<Vec<Entry>, RuntimeError>;
    async fn report(&self, owner: &Owner, report: Value) -> Result<(), RuntimeError>;
    async fn resolve(&self, owner: &Owner, request: Resolution) -> Result<(), RuntimeError>;
    async fn access(&self, owner: &Owner, device: &str, enabled: bool) -> Result<(), RuntimeError>;
}
