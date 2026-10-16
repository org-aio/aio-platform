use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 宿主不推断产品配置。开发宿主不提供发布源和默认组合即可完全离线启动。
#[derive(Clone)]
pub struct HostConfig {
    pub database_url: String,
    pub cache_root: PathBuf,
    pub public_origin: String,
    pub component_storage: Option<ComponentStorage>,
    pub default_plugins: Vec<PluginSource>,
    pub delivery: Option<DeliveryConfig>,
    pub development: Option<az_plugin_development::DevHostSession>,
    /// 部署方提供的远端能力绑定文件；其中只保存凭据环境变量名。
    pub remote_capabilities: Option<PathBuf>,
}

#[derive(Clone)]
pub struct ComponentStorage {
    pub database_url: String,
    pub root: PathBuf,
}

#[derive(Clone)]
pub struct DeliveryConfig {
    pub owner: String,
    pub discovery_interval_seconds: u64,
    pub revision_interval_seconds: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginSource {
    pub git: String,
    pub rev: String,
}
