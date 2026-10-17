use super::{
    BoundRuntime, DiscoveredPlugin, ExecutionAvailability, PageBinding, ProcessBinding,
    ProcessInstance, ProcessTarget, PublishJob, PublisherBinding, ServiceBinding, WasmTarget,
};
use crate::runtime::{
    MarketplaceEntry, PageBody, PageDefinition, PluginLifecycleEvent, PluginRuntime,
    PublishCredentialView, PublishState, RuntimeCatalog, UserView,
};
use anyhow::Result;
use az_plugin_package::PluginPackage;
use std::any::Any;

/// 旧版目录的租户安装与运行绑定；所有实现由同一个 PluginCatalog 实例提供。
#[async_trait::async_trait]
pub trait PluginInstallations: Any + Send + Sync {
    async fn has_plugins(&self, tenant_id: &str) -> Result<bool>;
    async fn source_id(&self, git: &str) -> Result<Option<String>>;
    async fn record_lifecycle_event(
        &self,
        tenant_id: &str,
        source_id: &str,
        revision_id: Option<&str>,
        lifecycle: &str,
        detail: &str,
    ) -> Result<()>;
    async fn lifecycle_events(
        &self,
        tenant_id: &str,
        source_id: &str,
    ) -> Result<Vec<PluginLifecycleEvent>>;
    async fn catalog(
        &self,
        tenant_id: &str,
        tenant_label: &str,
        user: UserView,
        availability: &ExecutionAvailability,
    ) -> Result<RuntimeCatalog>;
    async fn set_enabled(
        &self,
        tenant_id: &str,
        source_id: &str,
        enabled: bool,
        instance: Option<&ProcessInstance>,
    ) -> Result<()>;
    async fn uninstall(&self, tenant_id: &str, source_id: &str) -> Result<()>;
    async fn rollback_target(&self, tenant_id: &str, source_id: &str) -> Result<BoundRuntime>;
    async fn rollback_to(
        &self,
        tenant_id: &str,
        source_id: &str,
        target: &BoundRuntime,
        instance: Option<&ProcessInstance>,
    ) -> Result<()>;
    async fn active_service(
        &self,
        tenant_id: &str,
        source_id: &str,
    ) -> Result<Option<ServiceBinding>>;
    async fn active_page_binding(
        &self,
        tenant_id: &str,
        page_id: &str,
    ) -> Result<Option<PageBinding>>;
    async fn bound_runtime(&self, tenant_id: &str, source_id: &str) -> Result<BoundRuntime>;
    async fn active_wasm_revision(
        &self,
        tenant_id: &str,
        source_id: &str,
    ) -> Result<Option<String>>;
    async fn active_process(
        &self,
        tenant_id: &str,
        source_id: &str,
    ) -> Result<Option<ProcessBinding>>;
    async fn enabled_process_targets(&self) -> Result<Vec<ProcessTarget>>;
    async fn stop_orphan_process_records(&self) -> Result<()>;
    async fn enabled_wasm_targets(&self) -> Result<Vec<WasmTarget>>;
    async fn verify_revision_pages(
        &self,
        revision_id: &str,
        pages: &[PageDefinition],
    ) -> Result<()>;
    async fn recover_process_instance(
        &self,
        target: &ProcessTarget,
        instance: &ProcessInstance,
    ) -> Result<()>;
    async fn restore_process_instance(
        &self,
        tenant_id: &str,
        source_id: &str,
        target: &BoundRuntime,
        instance: &ProcessInstance,
    ) -> Result<()>;
    async fn recover_wasm_instance(&self, target: &WasmTarget) -> Result<()>;
}

/// 旧版目录的原子激活与仅发布提交；所有实现由同一个 PluginCatalog 实例提供。
#[async_trait::async_trait]
pub trait PluginActivation: Any + Send + Sync {
    async fn activate(
        &self,
        tenant_id: &str,
        plugin: DiscoveredPlugin,
        instance: Option<&ProcessInstance>,
        publication: Option<&MarketplaceEntry>,
    ) -> Result<()>;
    async fn publish_only(
        &self,
        tenant_id: &str,
        plugin: DiscoveredPlugin,
        publication: &MarketplaceEntry,
    ) -> Result<()>;
}

/// 旧版目录的不可变二进制包；所有实现由同一个 PluginCatalog 实例提供。
#[async_trait::async_trait]
pub trait PluginPackages: Any + Send + Sync {
    async fn save_package(&self, package: &PluginPackage) -> Result<()>;
    async fn package_archive(&self, revision: &str) -> Result<Option<Vec<u8>>>;
    async fn downloadable_package(&self, revision: &str) -> Result<Option<Vec<u8>>>;
    async fn published_package_entry(
        &self,
        git: &str,
        revision: &str,
    ) -> Result<Option<MarketplaceEntry>>;
}

/// 旧版目录的发布凭据与持久任务；所有实现由同一个 PluginCatalog 实例提供。
#[async_trait::async_trait]
pub trait PluginPublications: Any + Send + Sync {
    async fn create_publish_credential(
        &self,
        tenant_id: &str,
        git: &str,
    ) -> Result<PublishCredentialView>;
    async fn publisher_binding(&self, token: &str) -> Result<Option<PublisherBinding>>;
    async fn revoke_publish_credential(&self, tenant_id: &str, credential_id: &str) -> Result<()>;
    async fn queue_publish_job(
        &self,
        tenant_id: &str,
        source_id: &str,
        git: &str,
        revision: &str,
        runtime: PluginRuntime,
        page_count: usize,
    ) -> Result<PublishJob>;
    async fn claim_publish_job(&self, job_id: &str) -> Result<bool>;
    async fn finish_publish_job(
        &self,
        job_id: &str,
        state: PublishState,
        detail: &str,
        page_count: Option<usize>,
    ) -> Result<()>;
    async fn resume_publish_jobs(&self) -> Result<Vec<PublishJob>>;
    async fn publish_job(&self, tenant_id: &str, job_id: &str) -> Result<Option<PublishJob>>;
}

/// 旧版目录的已验证市场版本；所有实现由同一个 PluginCatalog 实例提供。
#[async_trait::async_trait]
pub trait PluginMarketplace: Any + Send + Sync {
    async fn published_marketplace_entry(
        &self,
        git: &str,
        requested_revision: Option<&str>,
    ) -> Result<Option<MarketplaceEntry>>;
    async fn marketplace_entries(&self, source: &str) -> Result<Vec<MarketplaceEntry>>;
}

/// 旧版目录的页面状态并发提交；所有实现由同一个 PluginCatalog 实例提供。
#[async_trait::async_trait]
pub trait PluginPageStates: Any + Send + Sync {
    async fn save_page_state(
        &self,
        tenant_id: &str,
        source_id: &str,
        revision_id: &str,
        page_id: &str,
        expected_generation: Option<i64>,
        body: &PageBody,
    ) -> Result<()>;
}
