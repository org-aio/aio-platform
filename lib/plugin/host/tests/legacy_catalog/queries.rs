use super::file::FileCatalog;
use anyhow::{Result, ensure};
use az_plugin_development::PublishedRelease;
use az_plugin_host::runtime::server::plugin_catalog::*;

#[async_trait::async_trait]
impl PluginCatalogQueries for FileCatalog {
    async fn source_git(&self, source: &str) -> Result<Option<String>> {
        Ok(self
            .read()
            .await?
            .revisions
            .iter()
            .find(|r| r.source == source)
            .map(|r| r.git.clone()))
    }
    async fn upgrade_eligible(&self, tenant: &str, source: &str, revision: &str) -> Result<bool> {
        let data = self.read().await?;
        let Ok(binding) = data.binding(tenant, source) else {
            return Ok(false);
        };
        let current = data.revision(source, &binding.revision)?;
        Ok(binding.enabled
            && binding.excluded.as_deref() != Some(revision)
            && data
                .publications
                .get(&current.git)
                .is_some_and(|p| p.rev == revision))
    }
    async fn publication_only(
        &self,
        _tenant: &str,
        _source: &str,
        _revision: &str,
    ) -> Result<bool> {
        // 此夹具不装配默认交付任务，因此没有交付恢复触发的仅发布意图。
        Ok(false)
    }
    async fn source_exists(&self, git: &str) -> Result<bool> {
        let data = self.read().await?;
        Ok(data.publications.contains_key(git) || data.revisions.iter().any(|r| r.git == git))
    }
    async fn mark_removed(&self, git: &str) -> Result<()> {
        self.change(|data| {
            data.removed.insert(git.into());
            Ok(())
        })
        .await
    }
    async fn removed_sources(&self) -> Result<Vec<String>> {
        Ok(self.read().await?.removed.into_iter().collect())
    }
    async fn is_removed(&self, git: &str) -> Result<bool> {
        Ok(self.read().await?.removed.contains(git))
    }
    async fn set_menu_hidden(&self, tenant: &str, source: &str, hidden: bool) -> Result<()> {
        self.change(|data| {
            let entries = data.hidden.entry(tenant.into()).or_default();
            if hidden {
                entries.insert(source.into());
            } else {
                entries.remove(source);
            }
            Ok(())
        })
        .await
    }
    async fn hidden_sources(&self, tenant: &str) -> Result<Vec<String>> {
        Ok(self
            .read()
            .await?
            .hidden
            .remove(tenant)
            .unwrap_or_default()
            .into_iter()
            .collect())
    }
    async fn hidden_page_ids(&self, tenant: &str, sources: &[String]) -> Result<Vec<String>> {
        let data = self.read().await?;
        let mut pages = vec![];
        for binding in data
            .bindings
            .iter()
            .filter(|b| b.tenant == tenant && b.enabled && sources.contains(&b.source))
        {
            pages.extend(
                data.revision(&binding.source, &binding.revision)?
                    .pages
                    .iter()
                    .map(|p| p.id.clone()),
            );
        }
        Ok(pages)
    }
    async fn releases(&self, git: &str) -> Result<Vec<PublishedRelease>> {
        let data = self.read().await?;
        let mut releases = vec![];
        for package in data
            .packages
            .iter()
            .rev()
            .filter(|p| {
                p.git == git
                    && data
                        .revisions
                        .iter()
                        .any(|r| r.git == p.git && r.revision == p.rev)
            })
            .take(100)
        {
            let Some(source_sha) = &package.source_revision else {
                continue;
            };
            ensure!(source_sha.len() == 40, "完整源码版本缺失");
            releases.push(PublishedRelease {
                git: git.into(),
                version: package.version.clone(),
                source_sha: source_sha.clone(),
                digest: package.rev.clone(),
                manifest: package.manifest_toml.clone(),
                abi: 1,
            });
        }
        Ok(releases)
    }
}
