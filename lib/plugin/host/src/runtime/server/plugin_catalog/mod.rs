//! 旧版目录的统一持久化契约；跨职责提交由同一个提供者保证原子性。

mod contract;
mod model;
#[cfg(feature = "postgres")]
mod postgres_queries;
mod queries;

pub use super::execution_availability::ExecutionAvailability;
pub use super::navigation::{NavigationLabelKind, NavigationLabelOverride};
pub use super::plugins::ProcessInstance;
pub use super::repository::DiscoveredPlugin;
#[cfg(feature = "postgres")]
pub use super::store::PluginStore;
pub use contract::*;
pub use model::*;
pub use queries::*;

/// 管理、发布、恢复与前端共享的目录。按职责拆分方法，但只允许一次整体绑定，
/// 不允许独立替换子契约而破坏发布、安装、页面状态之间的原子性。
/// 提供者不得将失败写入当作成功；宿主在失败时负责执行器补偿。
pub trait PluginCatalog:
    PluginInstallations
    + PluginActivation
    + PluginPackages
    + PluginPublications
    + PluginMarketplace
    + PluginPageStates
    + PluginCatalogQueries
{
}

/// 宿主发布包的目录来源，与具体持久化实现无关。
pub(crate) const PUBLISHED_REGISTRY_SOURCE: &str = "aio://published";
