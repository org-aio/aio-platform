//! 仅用于验收的文件提供者；同一实例串行化提交，重启后完全从文件读取。
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result, ensure};
use az_plugin_enhancement::Package;
use az_plugin_host::runtime::server::enhancement_catalog::*;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

#[derive(Clone, Default, Serialize, Deserialize)]
struct Tenant {
    head: Head,
    packages: BTreeMap<String, Package>,
    revisions: Vec<Revision>,
    events: Vec<Event>,
    observations: BTreeMap<i64, BTreeMap<String, Observation>>,
}

type Snapshot = BTreeMap<String, Tenant>;

pub struct FileCatalog {
    path: PathBuf,
    lock: Mutex<()>,
    pub fail_commit: AtomicBool,
}

impl FileCatalog {
    pub fn open(path: &Path) -> Arc<Self> {
        Arc::new(Self {
            path: path.into(),
            lock: Mutex::new(()),
            fail_commit: AtomicBool::new(false),
        })
    }

    fn read(&self) -> Result<Snapshot> {
        match std::fs::read(&self.path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Snapshot::default()),
            Err(error) => Err(error.into()),
        }
    }

    fn save(&self, snapshot: &Snapshot) -> Result<()> {
        let mut file =
            tempfile::NamedTempFile::new_in(self.path.parent().context("文件缺少父目录")?)?;
        serde_json::to_writer(file.as_file_mut(), snapshot)?;
        file.as_file().sync_all()?;
        ensure!(!self.fail_commit.load(Ordering::SeqCst), "模拟持久提交失败");
        file.persist(&self.path)?;
        Ok(())
    }
}

fn timestamp() -> Result<i64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}

#[async_trait::async_trait]
impl EnhancementCatalog for FileCatalog {
    async fn head(&self, tenant: &str) -> Result<Head> {
        Ok(self
            .read()?
            .get(tenant)
            .map(|value| value.head.clone())
            .unwrap_or_default())
    }

    async fn revisions(&self, tenant: &str) -> Result<Vec<Revision>> {
        let mut revisions = self
            .read()?
            .get(tenant)
            .map(|value| value.revisions.clone())
            .unwrap_or_default();
        revisions.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then(a.revision.cmp(&b.revision))
        });
        revisions.truncate(50);
        Ok(revisions)
    }

    async fn package(&self, tenant: &str, revision: &str) -> Result<Package> {
        self.read()?
            .get(tenant)
            .and_then(|value| value.packages.get(revision))
            .cloned()
            .context("当前租户没有该增强版本")
    }

    async fn propose(
        &self,
        tenant: &str,
        author: &str,
        package: &Package,
        authorize: CommitAuthorization<'_>,
    ) -> Result<()> {
        let _lock = self.lock.lock().await;
        let mut snapshot = self.read()?;
        let tenant = snapshot.entry(tenant.into()).or_default();
        let revision = package.revision()?;
        if !tenant.packages.contains_key(&revision) {
            tenant.packages.insert(revision.clone(), package.clone());
            tenant.revisions.push(Revision {
                revision,
                author: author.into(),
                created_at: timestamp()?,
            });
        }
        authorize.await?;
        self.save(&snapshot)
    }

    async fn activate(
        &self,
        tenant: &str,
        actor: &str,
        request: &Activation,
        rollback: bool,
        authorize: CommitAuthorization<'_>,
    ) -> Result<Option<Head>> {
        ensure!(request.expected_generation >= 0, "活动代数无效");
        let _lock = self.lock.lock().await;
        let mut snapshot = self.read()?;
        let tenant = snapshot.entry(tenant.into()).or_default();
        if tenant.head.generation != request.expected_generation {
            return Ok(None);
        }
        if rollback {
            let revision = request.revision.as_ref().context("回滚必须指定版本")?;
            ensure!(
                tenant
                    .events
                    .iter()
                    .any(|event| event.revision.as_ref() == Some(revision)),
                "只能回滚到曾激活的版本"
            );
        }
        if let Some(revision) = &request.revision {
            let candidate = tenant
                .packages
                .get(revision)
                .context("当前租户没有该增强版本")?;
            for event in &tenant.events {
                let Some(revision) = &event.revision else {
                    continue;
                };
                let previous = tenant.packages.get(revision).context("历史版本缺失")?;
                for old in &previous.enhancements {
                    if let Some(new) = candidate
                        .enhancements
                        .iter()
                        .find(|entry| entry.path == old.path)
                    {
                        let old = old.contract();
                        let new = new.contract();
                        ensure!(
                            old.input_schema == new.input_schema
                                && old.output_schema == new.output_schema,
                            "已发布路径不能修改 schema"
                        );
                    }
                }
            }
        }
        let next = Head {
            revision: request.revision.clone(),
            generation: tenant.head.generation.checked_add(1).context("代数溢出")?,
        };
        tenant.head = next.clone();
        tenant.events.push(Event {
            generation: next.generation,
            revision: next.revision.clone(),
            actor: actor.into(),
            action: match (&next.revision, rollback) {
                (None, _) => "deactivate",
                (_, true) => "rollback",
                _ => "activate",
            }
            .into(),
            created_at: timestamp()?,
        });
        authorize.await?;
        self.save(&snapshot)?;
        Ok(Some(next))
    }

    async fn history(&self, tenant: &str) -> Result<Vec<Event>> {
        Ok(self
            .read()?
            .get(tenant)
            .map(|value| value.events.iter().rev().take(50).cloned().collect())
            .unwrap_or_default())
    }

    async fn observe(
        &self,
        tenant: &str,
        revision: &str,
        generation: i64,
    ) -> Result<Vec<Observation>> {
        let snapshot = self.read()?;
        let tenant = snapshot.get(tenant).context("租户不存在")?;
        ensure!(
            tenant
                .events
                .iter()
                .any(|event| event.generation == generation
                    && event.revision.as_deref() == Some(revision)),
            "版本代数不属于当前租户"
        );
        Ok(tenant
            .observations
            .get(&generation)
            .map(|rows| rows.values().cloned().collect())
            .unwrap_or_default())
    }

    async fn started(
        &self,
        tenant: &str,
        revision: &str,
        generation: i64,
        path: &str,
    ) -> Result<()> {
        let _lock = self.lock.lock().await;
        let mut snapshot = self.read()?;
        let tenant = snapshot.get_mut(tenant).context("租户不存在")?;
        ensure!(
            tenant
                .events
                .iter()
                .any(|event| event.generation == generation
                    && event.revision.as_deref() == Some(revision)),
            "版本代数无效"
        );
        let observation = tenant
            .observations
            .entry(generation)
            .or_default()
            .entry(path.into())
            .or_insert(Observation {
                path: path.into(),
                started: 0,
                succeeded: 0,
                failed: 0,
                updated_at: 0,
            });
        observation.started += 1;
        observation.updated_at = timestamp()?;
        self.save(&snapshot)
    }

    async fn finished(
        &self,
        tenant: &str,
        generation: i64,
        path: &str,
        success: bool,
    ) -> Result<()> {
        let _lock = self.lock.lock().await;
        let mut snapshot = self.read()?;
        if let Some(observation) = snapshot
            .get_mut(tenant)
            .and_then(|tenant| tenant.observations.get_mut(&generation))
            .and_then(|rows| rows.get_mut(path))
        {
            observation.succeeded += i64::from(success);
            observation.failed += i64::from(!success);
            observation.updated_at = timestamp()?;
        }
        self.save(&snapshot)
    }
}
