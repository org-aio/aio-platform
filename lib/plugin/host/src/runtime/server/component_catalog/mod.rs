//! 整包目录的持久化边界；执行器、HTTP、首次引导与 Sidecar 共用同一实例。

mod model;
mod postgres;
mod postgres_rows;

use std::any::Any;

use anyhow::Result;
use az_plugin_bundle::Bundle;
use uuid::Uuid;

pub use model::*;
pub use postgres::PostgresComponentCatalog;

/// 原生提供者负责持久化和原子提交，宿主负责授权、整包校验及执行器补偿。
/// 所有写入错误必须返回；实现不能在失败后留下半个发布或首次安装标记。
#[async_trait::async_trait]
pub trait ComponentCatalog: Any + Send + Sync {
    async fn source(&self, git: &str) -> Result<Option<ComponentSource>>;
    async fn published(&self, git: &str, revision: Option<&str>) -> Result<Option<(Uuid, Bundle)>>;
    async fn contains_source(&self, source: Uuid) -> Result<bool>;
    async fn publish(&self, publication: ComponentPublication) -> Result<()>;
    async fn entries(&self, tenant: &str) -> Result<Vec<ComponentCatalogEntry>>;
    async fn installations(
        &self,
        tenant: Option<&str>,
        enabled_only: bool,
    ) -> Result<Vec<ComponentCatalogEntry>>;
    async fn installation(
        &self,
        tenant: &str,
        source: Uuid,
    ) -> Result<Option<ComponentCatalogEntry>>;
    async fn install(&self, commit: ComponentInstallationCommit) -> Result<()>;
    async fn remove(
        &self,
        tenant: &str,
        source: Uuid,
        generation: Uuid,
        uninstall: bool,
    ) -> Result<()>;
    async fn rollback_target(&self, tenant: &str, source: Uuid) -> Result<ComponentSelection>;
    async fn parent_enabled(&self, tenant: &str, parent: &str) -> Result<bool>;
    async fn children(&self, tenant: &str, source: Uuid) -> Result<Vec<String>>;
    async fn initialized(&self, tenant: &str, git: &str) -> Result<bool>;
    async fn details(&self, revision: &str) -> Result<Option<serde_json::Value>>;
    async fn archive(&self, revision: &str) -> Result<Option<Vec<u8>>>;
    async fn releases(&self, git: &str) -> Result<Vec<Bundle>>;
    async fn documentation_source(&self, revision: &str) -> Result<String>;
    async fn save_documentation(&self, revision: &str, readme: &str) -> Result<()>;
    /// 并发首次访问只能选定一个加密后的密钥；不得覆盖已有密钥。
    async fn process_key(&self, source: Uuid, tenant: &str, candidate: Vec<u8>) -> Result<Vec<u8>>;
    async fn record_process_actor(&self, source: Uuid, tenant: &str, user: &str) -> Result<()>;
    async fn process_actor_exists(&self, source: Uuid, tenant: &str, user: &str) -> Result<bool>;
    async fn rollouts(&self) -> Result<Vec<ComponentRollout>>;
    async fn upgrade_target(
        &self,
        tenant: &str,
        source: Uuid,
        digest: &str,
    ) -> Result<Option<String>>;
    async fn finish_rollout(
        &self,
        rollout: &ComponentRollout,
        result: ComponentRolloutResult,
    ) -> Result<()>;
}
