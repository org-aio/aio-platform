//! 测试用文件目录：每次读取重新打开文件，提交用同目录原子替换，不依赖 PostgreSQL。

use anyhow::{Context, Result, ensure};
use az_plugin_bundle::Bundle;
use az_plugin_host::runtime::server::plugins::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Default, Serialize, Deserialize)]
struct State {
    versions: Vec<Version>,
    publications: Vec<(Uuid, String)>,
    installations: Vec<(Uuid, ComponentInstallation)>,
    initialized: Vec<(String, String)>,
    keys: Vec<(Uuid, String, Vec<u8>)>,
    actors: Vec<(Uuid, String, String)>,
}
#[derive(Serialize, Deserialize)]
struct Version {
    source: ComponentSource,
    version: ComponentVersion,
    archive: Vec<u8>,
    readme: String,
}

pub struct FileCatalog {
    path: PathBuf,
    gate: Mutex<()>,
    pub reject_install: AtomicBool,
}
impl FileCatalog {
    pub fn open(path: &Path) -> Arc<Self> {
        Arc::new(Self {
            path: path.into(),
            gate: Mutex::new(()),
            reject_install: AtomicBool::new(false),
        })
    }
    fn read(&self) -> Result<State> {
        match std::fs::read(&self.path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
            Err(error) => Err(error.into()),
        }
    }
    fn save(&self, state: &State) -> Result<()> {
        let parent = self.path.parent().context("目录路径缺少父级")?;
        let temporary = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(temporary.as_file(), state)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&self.path)?;
        Ok(())
    }
}
fn entry(
    state: &State,
    version: &Version,
    installation: Option<&ComponentInstallation>,
) -> ComponentCatalogEntry {
    let menu_source = state
        .versions
        .iter()
        .find(|v| Some(&v.source.git) == version.source.parent.as_ref())
        .map(|v| v.source.id)
        .unwrap_or(version.source.id);
    ComponentCatalogEntry {
        source: version.source.clone(),
        menu_source,
        version: version.version.clone(),
        installation: installation.cloned(),
    }
}

#[async_trait::async_trait]
impl ComponentCatalog for FileCatalog {
    async fn source(&self, git: &str) -> Result<Option<ComponentSource>> {
        Ok(self
            .read()?
            .versions
            .iter()
            .find(|v| v.source.git == git)
            .map(|v| v.source.clone()))
    }
    async fn published(&self, git: &str, revision: Option<&str>) -> Result<Option<(Uuid, Bundle)>> {
        let state = self.read()?;
        let version = state.versions.iter().find(|v| {
            v.source.git == git
                && state.publications.iter().any(|(id, digest)| {
                    *id == v.source.id && v.version.digest == revision.unwrap_or(digest)
                })
        });
        version
            .map(|v| Ok((v.source.id, Bundle::decode(&v.archive)?)))
            .transpose()
    }
    async fn contains_source(&self, source: Uuid) -> Result<bool> {
        Ok(self
            .read()?
            .publications
            .iter()
            .any(|(id, _)| *id == source))
    }
    async fn publish(&self, publication: ComponentPublication) -> Result<()> {
        // 此测试目录没有交付任务后端，必须拒绝，不能伪造任务的原子完成。
        ensure!(
            publication.delivery_job.is_none(),
            "测试目录未配置交付任务存储"
        );
        let _guard = self.gate.lock().await;
        let mut state = self.read()?;
        let verified = publication.bundle.verify()?;
        let manifest = &verified.manifest().plugin;
        let metadata = manifest.marketplace.clone().context("缺少市场元数据")?;
        let source = ComponentSource {
            id: publication.source,
            git: publication.bundle.git.clone(),
            parent: metadata.parent.clone(),
        };
        if let Some(old) = state.versions.iter().find(|v| v.source.git == source.git) {
            ensure!(
                old.source.id == source.id && old.source.parent == source.parent,
                "来源已变化"
            );
        }
        if !state
            .versions
            .iter()
            .any(|v| v.version.digest == publication.bundle.digest)
        {
            state.versions.push(Version {
                source,
                version: ComponentVersion {
                    digest: publication.bundle.digest.clone(),
                    description: publication.description,
                    metadata,
                    capabilities: manifest.capabilities.clone(),
                    permissions: manifest.permissions.clone(),
                    required_permissions: manifest.required_permissions.clone(),
                },
                archive: publication.bundle.encode()?,
                readme: publication.readme,
            });
        }
        if let Some((_, digest)) = state
            .publications
            .iter_mut()
            .find(|(id, _)| *id == publication.source)
        {
            if publication.replace_publication {
                *digest = publication.bundle.digest;
            }
        } else {
            state
                .publications
                .push((publication.source, publication.bundle.digest));
        }
        self.save(&state)
    }
    async fn entries(&self, tenant: &str) -> Result<Vec<ComponentCatalogEntry>> {
        let state = self.read()?;
        let mut result = Vec::new();
        for (source, digest) in &state.publications {
            let version = state
                .versions
                .iter()
                .find(|v| &v.version.digest == digest)
                .context("版本缺失")?;
            let installation = state
                .installations
                .iter()
                .find(|(id, i)| id == source && i.tenant == tenant)
                .map(|(_, i)| i);
            result.push(entry(&state, version, installation));
        }
        Ok(result)
    }
    async fn installations(
        &self,
        tenant: Option<&str>,
        enabled_only: bool,
    ) -> Result<Vec<ComponentCatalogEntry>> {
        let state = self.read()?;
        state
            .installations
            .iter()
            .filter(|(_, i)| tenant.is_none_or(|t| t == i.tenant) && (!enabled_only || i.enabled))
            .map(|(id, i)| {
                let v = state
                    .versions
                    .iter()
                    .find(|v| v.source.id == *id && v.version.digest == i.digest)
                    .context("安装版本缺失")?;
                Ok(entry(&state, v, Some(i)))
            })
            .collect()
    }
    async fn installation(
        &self,
        tenant: &str,
        source: Uuid,
    ) -> Result<Option<ComponentCatalogEntry>> {
        Ok(self
            .installations(Some(tenant), false)
            .await?
            .into_iter()
            .find(|entry| entry.source.id == source))
    }
    async fn install(&self, commit: ComponentInstallationCommit) -> Result<()> {
        let _guard = self.gate.lock().await;
        let mut state = self.read()?;
        let previous = state
            .installations
            .iter()
            .find(|(id, i)| *id == commit.source && i.tenant == commit.tenant)
            .map(|(_, i)| i.generation);
        ensure!(previous == commit.expected_generation, "安装代数冲突");
        let git = state
            .versions
            .iter()
            .find(|v| v.source.id == commit.source && v.version.digest == commit.digest)
            .context("安装版本缺失")?
            .source
            .git
            .clone();
        state
            .installations
            .retain(|(id, i)| *id != commit.source || i.tenant != commit.tenant);
        state.installations.push((
            commit.source,
            ComponentInstallation {
                tenant: commit.tenant.clone(),
                digest: commit.digest,
                generation: Uuid::new_v4(),
                enabled: true,
                excluded_digest: commit.excluded_digest,
            },
        ));
        if !state
            .initialized
            .contains(&(commit.tenant.clone(), git.clone()))
        {
            state.initialized.push((commit.tenant, git));
        }
        ensure!(
            !self.reject_install.load(Ordering::SeqCst),
            "注入目录提交失败"
        );
        self.save(&state)
    }
    async fn remove(
        &self,
        tenant: &str,
        source: Uuid,
        generation: Uuid,
        uninstall: bool,
    ) -> Result<()> {
        let _guard = self.gate.lock().await;
        let mut state = self.read()?;
        let index = state
            .installations
            .iter()
            .position(|(id, i)| *id == source && i.tenant == tenant && i.generation == generation)
            .context("安装代数冲突")?;
        if uninstall {
            state.installations.remove(index);
        } else {
            state.installations[index].1.enabled = false;
            state.installations[index].1.generation = Uuid::new_v4();
        }
        self.save(&state)
    }
    async fn rollback_target(&self, tenant: &str, source: Uuid) -> Result<ComponentSelection> {
        let state = self.read()?;
        let (_, installation) = state
            .installations
            .iter()
            .find(|(id, i)| *id == source && i.tenant == tenant)
            .context("没有安装记录")?;
        let versions: Vec<_> = state
            .versions
            .iter()
            .filter(|v| v.source.id == source)
            .collect();
        let position = versions
            .iter()
            .position(|v| v.version.digest == installation.digest)
            .context("没有当前版本")?;
        let version = versions
            .get(position.checked_sub(1).context("没有回滚版本")?)
            .context("没有回滚版本")?;
        Ok(ComponentSelection {
            git: version.source.git.clone(),
            digest: version.version.digest.clone(),
            excluded_digest: state
                .publications
                .iter()
                .find(|(id, _)| *id == source)
                .map(|(_, digest)| digest.clone()),
        })
    }
    async fn parent_enabled(&self, tenant: &str, parent: &str) -> Result<bool> {
        Ok(self
            .installations(Some(tenant), true)
            .await?
            .iter()
            .any(|e| e.source.git == parent))
    }
    async fn children(&self, tenant: &str, source: Uuid) -> Result<Vec<String>> {
        let state = self.read()?;
        let git = state
            .versions
            .iter()
            .find(|v| v.source.id == source)
            .map(|v| &v.source.git);
        Ok(self
            .installations(Some(tenant), false)
            .await?
            .into_iter()
            .filter(|e| e.source.parent.as_ref() == git && git.is_some())
            .map(|e| e.version.metadata.title)
            .collect())
    }
    async fn initialized(&self, tenant: &str, git: &str) -> Result<bool> {
        Ok(self
            .read()?
            .initialized
            .contains(&(tenant.into(), git.into())))
    }
    async fn details(&self, revision: &str) -> Result<Option<Value>> {
        Ok(self
            .read()?
            .versions
            .iter()
            .find(|v| v.version.digest == revision)
            .map(|v| json!({"readme":v.readme,"versions":[],"builds":[]})))
    }
    async fn archive(&self, revision: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .read()?
            .versions
            .into_iter()
            .find(|v| v.version.digest == revision)
            .map(|v| v.archive))
    }
    async fn releases(&self, git: &str) -> Result<Vec<Bundle>> {
        self.read()?
            .versions
            .iter()
            .rev()
            .filter(|v| v.source.git == git)
            .map(|v| Bundle::decode(&v.archive))
            .collect()
    }
    async fn documentation_source(&self, revision: &str) -> Result<String> {
        self.read()?
            .versions
            .into_iter()
            .find(|v| v.version.digest == revision)
            .map(|v| v.source.git)
            .context("版本不存在")
    }
    async fn save_documentation(&self, revision: &str, readme: &str) -> Result<()> {
        let _guard = self.gate.lock().await;
        let mut state = self.read()?;
        let version = state
            .versions
            .iter_mut()
            .find(|v| v.version.digest == revision)
            .context("版本不存在")?;
        if version.readme.is_empty() {
            version.readme = readme.into();
        }
        self.save(&state)
    }
    async fn process_key(&self, source: Uuid, tenant: &str, candidate: Vec<u8>) -> Result<Vec<u8>> {
        let _guard = self.gate.lock().await;
        let mut state = self.read()?;
        if let Some((_, _, key)) = state
            .keys
            .iter()
            .find(|(id, t, _)| *id == source && t == tenant)
        {
            return Ok(key.clone());
        }
        state.keys.push((source, tenant.into(), candidate.clone()));
        self.save(&state)?;
        Ok(candidate)
    }
    async fn record_process_actor(&self, source: Uuid, tenant: &str, user: &str) -> Result<()> {
        let _guard = self.gate.lock().await;
        let mut state = self.read()?;
        let actor = (source, tenant.into(), user.into());
        if !state.actors.contains(&actor) {
            state.actors.push(actor);
        }
        self.save(&state)
    }
    async fn process_actor_exists(&self, source: Uuid, tenant: &str, user: &str) -> Result<bool> {
        Ok(self
            .read()?
            .actors
            .contains(&(source, tenant.into(), user.into())))
    }
    async fn rollouts(&self) -> Result<Vec<ComponentRollout>> {
        Ok(Vec::new())
    }
    async fn upgrade_target(
        &self,
        tenant: &str,
        source: Uuid,
        digest: &str,
    ) -> Result<Option<String>> {
        let state = self.read()?;
        Ok(state
            .installations
            .iter()
            .find(|(id, i)| {
                *id == source
                    && i.tenant == tenant
                    && i.enabled
                    && i.digest != digest
                    && i.excluded_digest.as_deref() != Some(digest)
            })
            .and_then(|_| {
                state
                    .versions
                    .iter()
                    .find(|v| v.source.id == source && v.version.digest == digest)
            })
            .map(|v| v.source.git.clone()))
    }
    async fn finish_rollout(&self, _: &ComponentRollout, _: ComponentRolloutResult) -> Result<()> {
        anyhow::bail!("测试目录未配置自动交付队列")
    }
}
