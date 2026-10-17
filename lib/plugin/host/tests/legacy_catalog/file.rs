//! 可持久化的替代目录夹具，仅接受页面与替代 Wasm；不作为正式后端发布。
use anyhow::{Context, Result, ensure};
use az_plugin_host::runtime::{server::plugin_catalog::*, *};
use az_plugin_package::PluginPackage;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Snapshot {
    pub packages: Vec<PluginPackage>,
    pub revisions: Vec<Revision>,
    pub bindings: Vec<Binding>,
    pub publications: BTreeMap<String, MarketplaceEntry>,
    pub jobs: Vec<Job>,
    pub credentials: Vec<Credential>,
    pub hidden: BTreeMap<String, BTreeSet<String>>,
    pub removed: BTreeSet<String>,
    pub pages: Vec<SavedPage>,
    pub events: Vec<(String, String, PluginLifecycleEvent)>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Revision {
    pub source: String,
    pub git: String,
    pub revision: String,
    pub runtime: PluginRuntime,
    pub manifest: serde_json::Value,
    pub pages: Vec<PageDefinition>,
    pub artifact: String,
}
impl Revision {
    pub fn id(&self) -> String {
        format!("{}:{}", self.source, self.revision)
    }
    pub fn target(&self) -> BoundRuntime {
        BoundRuntime {
            revision_id: self.id(),
            revision: self.revision.clone(),
            runtime: self.runtime,
            artifact: self.artifact.clone(),
        }
    }
    pub fn service(&self) -> Result<ServiceBinding> {
        let manifest: az_plugin_manifest::PluginManifest =
            serde_json::from_value(self.manifest.clone())?;
        Ok(ServiceBinding {
            runtime: self.runtime,
            revision: self.revision.clone(),
            endpoint: None,
            routes: manifest
                .subplugins
                .into_iter()
                .flat_map(|sub| sub.routes)
                .collect(),
        })
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Binding {
    pub tenant: String,
    pub source: String,
    pub revision: String,
    pub enabled: bool,
    pub generation: String,
    pub excluded: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct SavedPage {
    pub tenant: String,
    pub revision: String,
    pub page: String,
    pub body: PageBody,
    pub generation: i64,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Job {
    pub id: String,
    pub tenant: String,
    pub source: String,
    pub git: String,
    pub revision: String,
    pub runtime: PluginRuntime,
    pub pages: usize,
    pub state: PublishState,
    pub detail: String,
}
impl Job {
    pub fn view(&self) -> PublishJob {
        PublishJob {
            id: self.id.clone(),
            tenant_id: self.tenant.clone(),
            source_id: self.source.clone(),
            git: self.git.clone(),
            revision: self.revision.clone(),
            runtime: self.runtime,
            page_count: self.pages,
            state: self.state,
            detail: self.detail.clone(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Credential {
    pub id: String,
    pub tenant: String,
    pub git: String,
    pub hash: String,
}

pub struct FileCatalog {
    pub(super) path: PathBuf,
    lock: Mutex<()>,
    pub fail_commit: AtomicBool,
    pub fail_activation: AtomicBool,
}
impl FileCatalog {
    pub fn open(path: &Path) -> Arc<Self> {
        Arc::new(Self {
            path: path.into(),
            lock: Mutex::new(()),
            fail_commit: AtomicBool::new(false),
            fail_activation: AtomicBool::new(false),
        })
    }
    fn read_unlocked(&self) -> Result<Snapshot> {
        match std::fs::read(&self.path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Snapshot::default()),
            Err(error) => Err(error.into()),
        }
    }
    pub(super) async fn read(&self) -> Result<Snapshot> {
        let _guard = self.lock.lock().await;
        self.read_unlocked()
    }
    pub(super) async fn change<T>(
        &self,
        action: impl FnOnce(&mut Snapshot) -> Result<T>,
    ) -> Result<T> {
        let _guard = self.lock.lock().await;
        let mut data = self.read_unlocked()?;
        let result = action(&mut data)?;
        ensure!(
            !self.fail_commit.swap(false, Ordering::SeqCst),
            "测试目录提交失败"
        );
        let parent = self.path.parent().context("目录缺少父目录")?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        use std::io::Write;
        temporary.write_all(&serde_json::to_vec(&data)?)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&self.path)?;
        std::fs::File::open(parent)?.sync_all()?;
        Ok(result)
    }
}
impl PluginCatalog for FileCatalog {}
impl Snapshot {
    pub fn binding(&self, tenant: &str, source: &str) -> Result<&Binding> {
        self.bindings
            .iter()
            .find(|b| b.tenant == tenant && b.source == source)
            .context("安装不存在")
    }
    pub fn revision(&self, source: &str, revision: &str) -> Result<&Revision> {
        self.revisions
            .iter()
            .find(|r| r.source == source && r.revision == revision)
            .context("版本不存在")
    }
    pub fn event(&mut self, tenant: &str, source: &str, lifecycle: &str, detail: &str) {
        self.events.push((
            tenant.into(),
            source.into(),
            PluginLifecycleEvent {
                id: Uuid::new_v4().to_string(),
                lifecycle: lifecycle.into(),
                detail: detail.into(),
                created_at: self.events.len().to_string(),
            },
        ));
    }
    pub fn pages(&self, tenant: &str, revision: &Revision) -> Vec<PageDefinition> {
        let mut pages = revision.pages.clone();
        for page in &mut pages {
            if let Some(saved) = self
                .pages
                .iter()
                .find(|s| s.tenant == tenant && s.revision == revision.id() && s.page == page.id)
            {
                page.body = saved.body.clone();
            }
        }
        pages
    }
}

#[async_trait::async_trait]
impl PluginActivation for FileCatalog {
    async fn activate(
        &self,
        tenant: &str,
        plugin: DiscoveredPlugin,
        instance: Option<&ProcessInstance>,
        publication: Option<&MarketplaceEntry>,
    ) -> Result<()> {
        ensure!(instance.is_none(), "夹具不接受进程");
        self.commit_activation(tenant, plugin, publication, true)
            .await
    }
    async fn publish_only(
        &self,
        tenant: &str,
        plugin: DiscoveredPlugin,
        publication: &MarketplaceEntry,
    ) -> Result<()> {
        self.commit_activation(tenant, plugin, Some(publication), false)
            .await
    }
}
impl FileCatalog {
    async fn commit_activation(
        &self,
        tenant: &str,
        plugin: DiscoveredPlugin,
        publication: Option<&MarketplaceEntry>,
        bind: bool,
    ) -> Result<()> {
        ensure!(
            matches!(
                plugin.runtime,
                PluginRuntime::PageDefinition | PluginRuntime::WasmComponent
            ),
            "夹具只支持页面与 Wasm"
        );
        self.change(|data| {
            if publication.is_some() {
                let current = data
                    .jobs
                    .iter()
                    .rev()
                    .find(|j| j.tenant == tenant && j.git == plugin.git)
                    .context("发布任务缺失")?;
                ensure!(current.revision == plugin.revision, "发布已被后续请求替代");
            }
            if let Some(existing) = data.revisions.iter().find(|r| r.git == plugin.git) {
                ensure!(existing.source == plugin.source_id, "来源标识不一致");
            }
            let revision = Revision {
                source: plugin.source_id.clone(),
                git: plugin.git.clone(),
                revision: plugin.revision.clone(),
                runtime: plugin.runtime,
                manifest: plugin.manifest,
                pages: plugin.pages,
                artifact: plugin.artifact,
            };
            if let Some(existing) = data
                .revisions
                .iter_mut()
                .find(|r| r.source == revision.source && r.revision == revision.revision)
            {
                *existing = revision;
            } else {
                data.revisions.push(revision);
            }
            if bind {
                data.bindings
                    .retain(|b| b.tenant != tenant || b.source != plugin.source_id);
                data.bindings.push(Binding {
                    tenant: tenant.into(),
                    source: plugin.source_id.clone(),
                    revision: plugin.revision,
                    enabled: true,
                    generation: Uuid::new_v4().to_string(),
                    excluded: None,
                });
                data.event(tenant, &plugin.source_id, "activate", "目录原子提交");
            }
            if let Some(publication) = publication {
                data.publications.insert(plugin.git, publication.clone());
            }
            ensure!(
                !self.fail_activation.swap(false, Ordering::SeqCst),
                "测试激活提交失败"
            );
            Ok(())
        })
        .await
    }
}

#[async_trait::async_trait]
impl PluginPageStates for FileCatalog {
    async fn save_page_state(
        &self,
        tenant: &str,
        source: &str,
        revision: &str,
        page: &str,
        expected: Option<i64>,
        body: &PageBody,
    ) -> Result<()> {
        self.change(|data| {
            let binding = data.binding(tenant, source)?;
            ensure!(
                binding.enabled && data.revision(source, &binding.revision)?.id() == revision,
                "页面版本已切换"
            );
            ensure!(
                data.revision(source, &binding.revision)?
                    .pages
                    .iter()
                    .any(|p| p.id == page),
                "页面不存在"
            );
            let previous = data
                .pages
                .iter_mut()
                .find(|p| p.tenant == tenant && p.revision == revision && p.page == page);
            ensure!(
                previous.as_ref().map(|p| p.generation) == expected,
                "页面状态已变化"
            );
            if let Some(previous) = previous {
                previous.body = body.clone();
                previous.generation += 1;
            } else {
                data.pages.push(SavedPage {
                    tenant: tenant.into(),
                    revision: revision.into(),
                    page: page.into(),
                    body: body.clone(),
                    generation: 1,
                });
            }
            Ok(())
        })
        .await
    }
}
