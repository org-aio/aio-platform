use anyhow::{Context as _, Result, ensure};
use az_plugin_manifest::{read_manifest, validate_host_compatibility};
use az_plugin_package::{PluginPackage, VerifiedPluginPackage};

use super::{
    remote_access::validate_git,
    repository::{RepositoryInstaller, is_artifact_revision},
};
use crate::runtime::{MarketplaceEntry, PluginRuntime};

pub(super) fn validate_publish_payload(package: &PluginPackage) -> Result<VerifiedPluginPackage> {
    let verified = package.verify()?;
    validate_git(&package.git)?;
    validate_host_compatibility(&verified.manifest, env!("CARGO_PKG_VERSION"))?;
    let runtime = verified
        .manifest
        .plugin
        .runtime
        .as_ref()
        .context("插件包缺少运行目标")?;
    ensure_publish_runtime(runtime.kind)?;
    ensure_publish_capabilities(&verified.manifest)?;
    Ok(verified)
}

pub(super) fn ensure_publish_runtime(runtime: PluginRuntime) -> Result<()> {
    ensure!(
        matches!(
            runtime,
            PluginRuntime::PageDefinition | PluginRuntime::WasmComponent | PluginRuntime::Process
        ),
        "发布接口只接受独立运行的 page-definition、wasm-component 或 process 产物"
    );
    Ok(())
}

pub(super) fn ensure_publish_capabilities(
    manifest: &az_plugin_manifest::RepositoryManifest,
) -> Result<()> {
    let capabilities = &manifest.plugin.capabilities;
    ensure!(
        capabilities.network.is_empty()
            && capabilities.filesystem.is_empty()
            && !capabilities.database,
        "当前在线发布宿主未授予网络、文件系统或数据库能力"
    );
    Ok(())
}

impl RepositoryInstaller {
    pub fn published_marketplace_entry(
        &self,
        git: &str,
        revision: &str,
    ) -> Result<MarketplaceEntry> {
        validate_git(git)?;
        ensure!(
            is_artifact_revision(revision),
            "插件版本必须是完整 Git SHA 或插件包 SHA-256"
        );
        let manifest = read_manifest(&self.cache_root.join(revision))?;
        let metadata = manifest
            .plugin
            .marketplace
            .as_ref()
            .context("在线发布插件必须声明 [plugin.marketplace]")?;
        Ok(MarketplaceEntry {
            parent_git: None,
            parent_title: None,
            git: git.to_owned(),
            rev: revision.to_owned(),
            title: metadata.title.clone(),
            summary: metadata.summary.clone(),
            license: metadata.license.clone(),
            tags: metadata.tags.clone(),
            installed: false,
            source_id: None,
            state: None,
            active_revision: None,
            runtime: manifest.plugin.runtime.map(|runtime| runtime.kind),
            capabilities: manifest.plugin.capabilities,
        })
    }
}
