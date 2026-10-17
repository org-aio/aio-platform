use super::file::{Credential, FileCatalog, Job};
use anyhow::{Context, Result, ensure};
use az_plugin_host::runtime::{server::plugin_catalog::*, *};
use az_plugin_package::PluginPackage;
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[async_trait::async_trait]
impl PluginPackages for FileCatalog {
    async fn save_package(&self, package: &PluginPackage) -> Result<()> {
        package.verify()?;
        self.change(|data| {
            if let Some(existing) = data
                .packages
                .iter()
                .find(|p| p.git == package.git && p.version == package.version)
            {
                ensure!(existing.rev == package.rev, "版本内容冲突");
            } else {
                data.packages.push(package.clone());
            }
            Ok(())
        })
        .await
    }
    async fn package_archive(&self, revision: &str) -> Result<Option<Vec<u8>>> {
        self.read()
            .await?
            .packages
            .iter()
            .find(|p| p.rev == revision)
            .map(|p| p.encode())
            .transpose()
    }
    async fn downloadable_package(&self, revision: &str) -> Result<Option<Vec<u8>>> {
        let data = self.read().await?;
        data.packages
            .iter()
            .find(|p| {
                p.rev == revision
                    && data
                        .revisions
                        .iter()
                        .any(|r| r.git == p.git && r.revision == p.rev)
            })
            .map(|p| p.encode())
            .transpose()
    }
    async fn published_package_entry(
        &self,
        git: &str,
        revision: &str,
    ) -> Result<Option<MarketplaceEntry>> {
        let data = self.read().await?;
        let Some(package) = data.packages.iter().find(|p| {
            p.git == git
                && p.rev == revision
                && data
                    .revisions
                    .iter()
                    .any(|r| r.git == p.git && r.revision == p.rev)
        }) else {
            return Ok(None);
        };
        let manifest = package.verify()?.manifest;
        let metadata = manifest.plugin.marketplace.context("市场元数据缺失")?;
        Ok(Some(MarketplaceEntry {
            parent_git: None,
            parent_title: None,
            git: git.into(),
            rev: revision.into(),
            title: metadata.title,
            summary: metadata.summary,
            license: metadata.license,
            tags: metadata.tags,
            menu_hidden: false,
            installed: false,
            source_id: None,
            state: None,
            active_revision: None,
            runtime: manifest.plugin.runtime.map(|r| r.kind),
            capabilities: manifest.plugin.capabilities,
        }))
    }
}
#[async_trait::async_trait]
impl PluginMarketplace for FileCatalog {
    async fn published_marketplace_entry(
        &self,
        git: &str,
        requested: Option<&str>,
    ) -> Result<Option<MarketplaceEntry>> {
        if let Some(revision) = requested {
            return self.published_package_entry(git, revision).await;
        }
        Ok(self.read().await?.publications.get(git).cloned())
    }
    async fn marketplace_entries(&self, source: &str) -> Result<Vec<MarketplaceEntry>> {
        ensure!(source == "aio://published", "夹具没有外部目录");
        Ok(self.read().await?.publications.into_values().collect())
    }
}
#[async_trait::async_trait]
impl PluginPublications for FileCatalog {
    async fn create_publish_credential(
        &self,
        tenant: &str,
        git: &str,
    ) -> Result<PublishCredentialView> {
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let id = Uuid::new_v4().to_string();
        self.change(|data| {
            data.credentials
                .retain(|c| c.tenant != tenant || c.git != git);
            data.credentials.push(Credential {
                id: id.clone(),
                tenant: tenant.into(),
                git: git.into(),
                hash: hash(&token),
            });
            Ok(())
        })
        .await?;
        Ok(PublishCredentialView {
            id,
            tenant_id: tenant.into(),
            git: git.into(),
            token,
        })
    }
    async fn publisher_binding(&self, token: &str) -> Result<Option<PublisherBinding>> {
        Ok(self
            .read()
            .await?
            .credentials
            .iter()
            .find(|c| c.hash == hash(token))
            .map(|c| PublisherBinding {
                tenant_id: c.tenant.clone(),
                git: c.git.clone(),
            }))
    }
    async fn revoke_publish_credential(&self, tenant: &str, id: &str) -> Result<()> {
        self.change(|data| {
            ensure!(
                data.credentials
                    .iter()
                    .any(|c| c.tenant == tenant && c.id == id),
                "凭据不存在"
            );
            data.credentials
                .retain(|c| c.tenant != tenant || c.id != id);
            Ok(())
        })
        .await
    }
    async fn queue_publish_job(
        &self,
        tenant: &str,
        source: &str,
        git: &str,
        revision: &str,
        runtime: PluginRuntime,
        pages: usize,
    ) -> Result<PublishJob> {
        self.change(|data| {
            let mut job = if let Some(index) = data
                .jobs
                .iter()
                .position(|j| j.tenant == tenant && j.git == git && j.revision == revision)
            {
                data.jobs.remove(index)
            } else {
                Job {
                    id: Uuid::new_v4().to_string(),
                    tenant: tenant.into(),
                    source: source.into(),
                    git: git.into(),
                    revision: revision.into(),
                    runtime,
                    pages,
                    state: PublishState::Queued,
                    detail: String::new(),
                }
            };
            if job.state != PublishState::Running {
                job.state = PublishState::Queued;
                job.detail = "等待验证".into();
                job.source = source.into();
                job.pages = pages;
                job.runtime = runtime;
            }
            let result = job.view();
            data.jobs.push(job);
            Ok(result)
        })
        .await
    }
    async fn claim_publish_job(&self, id: &str) -> Result<bool> {
        self.change(|data| {
            let Some(job) = data
                .jobs
                .iter_mut()
                .find(|j| j.id == id && j.state == PublishState::Queued)
            else {
                return Ok(false);
            };
            job.state = PublishState::Running;
            Ok(true)
        })
        .await
    }
    async fn finish_publish_job(
        &self,
        id: &str,
        state: PublishState,
        detail: &str,
        pages: Option<usize>,
    ) -> Result<()> {
        ensure!(
            matches!(state, PublishState::Active | PublishState::Failed),
            "任务不能以此状态结束"
        );
        self.change(|data| {
            let job = data
                .jobs
                .iter_mut()
                .find(|j| j.id == id && j.state == PublishState::Running)
                .context("运行任务不存在")?;
            job.state = state;
            job.detail = detail.into();
            if let Some(pages) = pages {
                job.pages = pages;
            }
            Ok(())
        })
        .await
    }
    async fn resume_publish_jobs(&self) -> Result<Vec<PublishJob>> {
        self.change(|data| {
            for job in &mut data.jobs {
                if job.state == PublishState::Running {
                    job.state = PublishState::Queued;
                }
            }
            Ok(data
                .jobs
                .iter()
                .filter(|j| j.state == PublishState::Queued)
                .map(Job::view)
                .collect())
        })
        .await
    }
    async fn publish_job(&self, tenant: &str, id: &str) -> Result<Option<PublishJob>> {
        Ok(self
            .read()
            .await?
            .jobs
            .iter()
            .find(|j| j.tenant == tenant && j.id == id)
            .map(Job::view))
    }
}
fn hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}
