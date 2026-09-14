use serde::{Deserialize, Serialize};

pub use az_plugin_manifest::{
    CapabilityManifest, PageActionResult, PageBody, PageDefinition, PluginRuntime,
};
#[cfg(feature = "server")]
pub use az_plugin_manifest::{ComponentResponse, PluginRequest};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageActionRequest {
    pub page_id: String,
    pub action_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeCatalog {
    pub session_context: String,
    pub context: String,
    pub page_versions: std::collections::BTreeMap<String, String>,
    pub tenant: TenantView,
    pub user: UserView,
    pub pages: Vec<PageDefinition>,
    #[serde(default)]
    pub account_items: Vec<RuntimeAccountItem>,
    pub plugins: Vec<InstalledPluginView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TenantView {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UserView {
    pub label: String,
    pub handle: String,
    pub initials: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeAccountItem {
    pub id: String,
    pub label: String,
    pub icon: Option<String>,
    pub page_id: String,
    #[serde(default)]
    pub required_permission: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg(feature = "server")]
pub struct PluginLifecycleEvent {
    pub id: String,
    pub lifecycle: String,
    pub detail: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstalledPluginView {
    pub source_id: String,
    pub git: String,
    pub revision: String,
    pub runtime: PluginRuntime,
    pub state: PluginState,
    #[serde(default)]
    pub capabilities: CapabilityManifest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginState {
    Active,
    Disabled,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg(feature = "server")]
pub enum PublishState {
    Queued,
    Running,
    Active,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg(feature = "server")]
pub struct InstallPluginRequest {
    pub git: String,
    pub rev: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg(feature = "server")]
#[serde(deny_unknown_fields)]
pub struct CreatePublishCredentialRequest {
    pub git: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg(feature = "server")]
pub struct PublishCredentialView {
    pub id: String,
    pub tenant_id: String,
    pub git: String,
    pub token: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg(feature = "server")]
pub struct PublishedPluginView {
    pub job_id: String,
    pub tenant_id: String,
    pub source_id: String,
    pub revision: String,
    pub runtime: PluginRuntime,
    pub page_count: usize,
    pub state: PublishState,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg(feature = "server")]
pub struct MarketplaceEntry {
    #[serde(default)]
    pub parent_git: Option<String>,
    #[serde(default)]
    pub parent_title: Option<String>,
    pub git: String,
    pub rev: String,
    pub title: String,
    pub summary: String,
    pub license: String,
    pub tags: Vec<String>,
    #[serde(default)]
    pub installed: bool,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub state: Option<PluginState>,
    #[serde(default)]
    pub active_revision: Option<String>,
    #[serde(default)]
    pub runtime: Option<PluginRuntime>,
    #[serde(default)]
    pub capabilities: CapabilityManifest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeResponse<T> {
    pub data: T,
}

#[cfg(test)]
mod tests {
    use super::PageActionRequest;

    #[test]
    fn page_action_request_rejects_spoofed_context() {
        let result = serde_json::from_str::<PageActionRequest>(
            r#"{"page_id":"counter","action_id":"increment","tenant_id":"spoofed"}"#,
        );

        assert!(result.is_err());
    }
}
