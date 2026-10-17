use az_plugin_bundle::{Bundle, MarketplaceManifest};
use az_plugin_contract::CapabilityGrants;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::super::plugins::ComponentDescription;

/// 来源是持久业务身份；执行实现的注册身份仍由 Dill 的 TypeId 决定。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComponentSource {
    pub id: Uuid,
    pub git: String,
    pub parent: Option<String>,
}

/// 已验证版本的轻量索引；目录读取不加载整包二进制。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComponentVersion {
    pub digest: String,
    pub description: ComponentDescription,
    pub metadata: MarketplaceManifest,
    pub capabilities: CapabilityGrants,
    pub permissions: Vec<String>,
    pub required_permissions: Vec<String>,
}

/// 租户安装意图与代数，独立于当前执行器是否可用。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComponentInstallation {
    pub tenant: String,
    pub digest: String,
    pub generation: Uuid,
    pub enabled: bool,
    pub excluded_digest: Option<String>,
}

/// 同一读取快照中的来源、版本和可选安装记录，供目录及权限判断使用。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComponentCatalogEntry {
    pub source: ComponentSource,
    pub menu_source: Uuid,
    pub version: ComponentVersion,
    pub installation: Option<ComponentInstallation>,
}

/// 发布事务同时保存不可变整包、索引和可选交付任务结果。
pub struct ComponentPublication {
    pub source: Uuid,
    pub bundle: Bundle,
    pub description: ComponentDescription,
    pub readme: String,
    pub delivery_job: Option<i64>,
    pub replace_publication: bool,
}

/// 安装提交必须原子更新安装记录与首次引导标记，并拒绝覆盖不同代数。
pub struct ComponentInstallationCommit {
    pub tenant: String,
    pub source: Uuid,
    pub digest: String,
    pub excluded_digest: Option<String>,
    pub expected_generation: Option<Uuid>,
}

/// 回滚或重新启用时使用已持久化的明确版本，不隐式升级。
pub struct ComponentSelection {
    pub git: String,
    pub digest: String,
    pub excluded_digest: Option<String>,
}

/// 升级队列记录保留当前安装版本，缺失执行器时不能消耗任务。
pub struct ComponentRollout {
    pub tenant: String,
    pub source: Uuid,
    pub digest: String,
    pub installed_digest: String,
}

/// 升级结果由目录实现持久化；取消或过时不能伪装成激活成功。
pub enum ComponentRolloutResult {
    Active,
    Superseded,
    Failed(String),
}
