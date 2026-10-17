//! 替代交付器与目录共用一次文件提交，仅支持页面包；不作为生产调度器。
use super::file::{FileCatalog, Revision};
use anyhow::{Context, Result, ensure};
use az_plugin_delivery::{BuildJob, BuildRecipe, BuildReport, DocumentImage, Documentation};
use az_plugin_host::runtime::{MarketplaceEntry, PluginRuntime, server::delivery::*};
use az_plugin_package::PluginPackage;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Build {
    job: BuildJob,
    state: String,
    until: u64,
    error: Option<String>,
    package: Option<PluginPackage>,
}
fn now() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs())
}
impl Build {
    fn valid(&self, lease: &str, now: u64) -> bool {
        self.job.lease == lease
            && self.until > now
            && matches!(self.state.as_str(), "building" | "uploaded")
    }
}
impl FileCatalog {
    pub(super) async fn enqueue(&self, git: &str, revision: &str) -> Result<()> {
        self.change(|data| {
            for old in data.builds.iter_mut().filter(|b| b.job.git == git) {
                old.state = "superseded".into();
                old.until = 0;
            }
            let id = data.builds.len() as i64 + 1;
            data.builds.push(Build {
                job: BuildJob {
                    id,
                    git: git.into(),
                    source_revision: revision.into(),
                    lease: String::new(),
                    version: format!("0.0.0-dev.{id}+{revision}"),
                    recipe: serde_json::from_value::<BuildRecipe>(
                        json!({"environment":"typescript","command":["fixture-build"]}),
                    )?,
                },
                state: "queued".into(),
                until: 0,
                error: None,
                package: None,
            });
            Ok(())
        })
        .await
    }
}
#[async_trait::async_trait]
impl DeliveryService for FileCatalog {
    async fn claim(&self) -> Result<Option<BuildJob>> {
        let now = now()?;
        self.change(|data| {
            if data.builds.iter().any(|b| b.valid(&b.job.lease, now)) {
                return Ok(None);
            }
            let Some(build) = data.builds.iter_mut().find(|b| {
                b.state == "queued"
                    || (matches!(b.state.as_str(), "building" | "uploaded") && b.until <= now)
            }) else {
                return Ok(None);
            };
            build.job.lease = Uuid::new_v4().to_string();
            build.state = if build.package.is_some() {
                "uploaded"
            } else {
                "building"
            }
            .into();
            build.until = now + 300;
            Ok(Some(build.job.clone()))
        })
        .await
    }
    async fn retained_jobs(&self) -> Result<Vec<i64>> {
        Ok(self
            .read()
            .await?
            .builds
            .iter()
            .filter(|b| matches!(b.state.as_str(), "queued" | "building" | "uploaded"))
            .map(|b| b.job.id)
            .collect())
    }
    async fn defer(&self, id: i64, lease: &str, error: &str) -> Result<bool> {
        let now = now()?;
        self.change(|data| {
            let Some(build) = data
                .builds
                .iter_mut()
                .find(|b| b.job.id == id && b.valid(lease, now))
            else {
                return Ok(false);
            };
            build.state = "queued".into();
            build.until = 0;
            build.error = Some(error.chars().take(16000).collect());
            Ok(true)
        })
        .await
    }
    async fn heartbeat(&self, id: i64, lease: &str) -> Result<bool> {
        let now = now()?;
        self.change(|data| {
            let Some(build) = data
                .builds
                .iter_mut()
                .find(|b| b.job.id == id && b.valid(lease, now))
            else {
                return Ok(false);
            };
            build.until = now + 300;
            Ok(true)
        })
        .await
    }
    async fn upload(
        &self,
        id: i64,
        lease: &str,
        format: DeliveryArtifact,
        bytes: &[u8],
    ) -> Result<()> {
        ensure!(
            matches!(format, DeliveryArtifact::Plugin),
            "文件交付夹具不支持 v2 整包"
        );
        let package = PluginPackage::decode(bytes)?;
        let verified = package.verify()?;
        ensure!(
            verified
                .manifest
                .plugin
                .runtime
                .context("运行目标缺失")?
                .kind
                == PluginRuntime::PageDefinition,
            "文件交付夹具只接受页面包"
        );
        let _: Vec<az_plugin_host::runtime::PageDefinition> =
            serde_json::from_slice(&verified.artifact)?;
        let now = now()?;
        self.change(|data| {
            let build = data
                .builds
                .iter_mut()
                .find(|b| b.job.id == id && b.valid(lease, now))
                .context("租约失效")?;
            ensure!(
                package.git == build.job.git
                    && package.version == build.job.version
                    && package.source_revision.as_deref() == Some(&build.job.source_revision),
                "包与任务不匹配"
            );
            build.package = Some(package);
            build.state = "uploaded".into();
            build.until = now + 300;
            Ok(())
        })
        .await
    }
    async fn complete(&self, id: i64, report: BuildReport) -> Result<()> {
        validate_documentation(&report.documentation)?;
        let now = now()?;
        self.change(|data| {
            let build = data
                .builds
                .iter_mut()
                .find(|b| b.job.id == id && b.valid(&report.lease, now))
                .context("租约失效")?;
            if let Some(error) = report.error {
                build.state = "failed".into();
                build.error = Some(error.chars().take(16000).collect());
                build.until = 0;
                return Ok(());
            }
            let package = build.package.clone().context("尚未上传包")?;
            build.state = "active".into();
            build.until = 0;
            let verified = package.verify()?;
            let metadata = verified
                .manifest
                .plugin
                .marketplace
                .clone()
                .context("缺少元数据")?;
            let source = data
                .revisions
                .iter()
                .find(|r| r.git == package.git)
                .map(|r| r.source.clone())
                .unwrap_or_else(|| Uuid::new_v4().to_string());
            let entry = MarketplaceEntry {
                git: package.git.clone(),
                rev: package.rev.clone(),
                title: metadata.title,
                summary: metadata.summary,
                license: metadata.license,
                tags: metadata.tags,
                parent_git: None,
                parent_title: None,
                menu_hidden: false,
                installed: false,
                source_id: None,
                state: None,
                active_revision: None,
                runtime: Some(PluginRuntime::PageDefinition),
                capabilities: verified.manifest.plugin.capabilities.clone(),
            };
            let revision = Revision {
                source,
                git: package.git.clone(),
                revision: package.rev.clone(),
                runtime: PluginRuntime::PageDefinition,
                manifest: serde_json::to_value(&verified.manifest.plugin)?,
                pages: serde_json::from_slice(&verified.artifact)?,
                artifact: "pages.json".into(),
            };
            // 同一文件原子提交包、文档、发布索引和任务完成；提交失败不会留下可见候选。
            data.documents
                .entry(package.rev.clone())
                .or_insert(report.documentation);
            data.publications.insert(package.git.clone(), entry);
            data.revisions.push(revision);
            data.packages.push(package);
            Ok(())
        })
        .await
    }
    async fn retry(&self, id: i64) -> Result<bool> {
        self.change(|data| {
            let Some(build) = data
                .builds
                .iter_mut()
                .find(|b| b.job.id == id && b.state == "failed")
            else {
                return Ok(false);
            };
            build.state = "queued".into();
            build.error = None;
            Ok(true)
        })
        .await
    }
    async fn builds(&self, git: &str) -> Result<Vec<Value>> {
        Ok(self.read().await?.builds.iter().rev().filter(|b| b.job.git == git).take(10).map(|b| json!({"id":b.job.id,"source_revision":b.job.source_revision,"state":b.state,"error":b.error})).collect())
    }
}
#[async_trait::async_trait]
impl PluginDocumentation for FileCatalog {
    async fn save(&self, revision: &str, doc: &Documentation) -> Result<()> {
        validate_documentation(doc)?;
        self.change(|data| {
            data.documents
                .entry(revision.into())
                .or_insert_with(|| doc.clone());
            Ok(())
        })
        .await
    }
    async fn details(&self, revision: &str) -> Result<Option<Value>> {
        let data = self.read().await?;
        if !data.revisions.iter().any(|r| r.revision == revision) {
            return Ok(None);
        }
        let Some(package) = data.packages.iter().find(|p| p.rev == revision) else {
            return Ok(None);
        };
        let versions: Vec<Value> = data
            .packages
            .iter()
            .filter(|p| p.git == package.git && data.revisions.iter().any(|r| r.revision == p.rev))
            .map(|p| json!({"revision":p.rev,"version":p.version}))
            .collect();
        Ok(Some(
            json!({"readme":data.documents.get(revision).map(|d| d.readme.as_str()).unwrap_or_default(),"version":package.version,"git":package.git,"source_revision":package.source_revision,"versions":versions}),
        ))
    }
    async fn image(&self, revision: &str, path: &str) -> Result<Option<DocumentImage>> {
        let data = self.read().await?;
        if !data.revisions.iter().any(|r| r.revision == revision) {
            return Ok(None);
        }
        Ok(data
            .documents
            .get(revision)
            .and_then(|d| d.images.get(path))
            .cloned())
    }
}
