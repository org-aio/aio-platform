#[cfg(feature = "postgres")]
mod controller;
#[cfg(feature = "postgres")]
mod devices;
#[cfg(feature = "postgres")]
mod documents;
mod model;
#[cfg(feature = "postgres")]
mod publication;
#[cfg(all(test, feature = "postgres"))]
mod registration_tests;
#[cfg(feature = "postgres")]
mod storage;
#[cfg(all(test, feature = "postgres"))]
mod tests;

#[cfg(feature = "postgres")]
mod routes;
pub(super) use model::MarketplaceItem;
#[cfg(feature = "postgres")]
pub(super) use routes::router;
#[cfg(feature = "postgres")]
pub(super) use storage::{PostgresToolCatalog, migrate};

/// CLI 工具目录贡献者；管理目录仅在已装配时聚合其条目。
#[async_trait::async_trait]
pub trait ToolCatalog: std::any::Any + Send + Sync {
    async fn entries(&self) -> anyhow::Result<Vec<az_tool::ToolManifest>>;
}
