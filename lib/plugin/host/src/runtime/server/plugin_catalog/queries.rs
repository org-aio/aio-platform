use std::any::Any;

use anyhow::Result;
use az_plugin_development::PublishedRelease;

/// 安装决策与目录偏好；默认提供者在提交时仍须复核任务版本，查询不是提交授权。
#[async_trait::async_trait]
pub trait PluginCatalogQueries: Any + Send + Sync {
    async fn source_git(&self, source: &str) -> Result<Option<String>>;
    async fn upgrade_eligible(&self, tenant: &str, source: &str, revision: &str) -> Result<bool>;
    async fn publication_only(&self, tenant: &str, source: &str, revision: &str) -> Result<bool>;
    async fn source_exists(&self, git: &str) -> Result<bool>;
    async fn mark_removed(&self, git: &str) -> Result<()>;
    async fn removed_sources(&self) -> Result<Vec<String>>;
    async fn is_removed(&self, git: &str) -> Result<bool>;
    async fn set_menu_hidden(&self, tenant: &str, source: &str, hidden: bool) -> Result<()>;
    async fn hidden_sources(&self, tenant: &str) -> Result<Vec<String>>;
    async fn hidden_page_ids(&self, tenant: &str, sources: &[String]) -> Result<Vec<String>>;
    async fn releases(&self, git: &str) -> Result<Vec<PublishedRelease>>;
}
