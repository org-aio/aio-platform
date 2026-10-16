use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use arc_swap::ArcSwap;
use serde_json::Value;

use super::super::plugins::{
    CapabilityProvider, CapabilityRequest, PluginTasks, RuntimeCapabilities, RuntimeCapability,
};
use super::{bindings::Bindings, configuration::Loaded};

/// Dill 中只有一个管理实例；每次发布生成不可变绑定快照，已有调用继续持有旧快照。
pub(in crate::runtime::server) struct RemoteCapabilities {
    active: ArcSwap<Version>,
    stopped: Arc<AtomicBool>,
    tasks: Arc<PluginTasks>,
    publication: tokio::sync::Mutex<()>,
}

struct Version {
    revision: String,
    generation: u64,
    bindings: Arc<Bindings>,
}

impl RemoteCapabilities {
    pub async fn open(path: &Path, tasks: Arc<PluginTasks>) -> Result<Self> {
        let loaded = Loaded::read(path).await?;
        let revision = loaded.revision.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let bindings = Bindings::open(loaded, tasks.clone(), stopped.clone()).await?;
        let active = ArcSwap::from_pointee(Version {
            revision,
            generation: 1,
            bindings: Arc::new(bindings),
        });
        Ok(Self {
            active,
            stopped,
            tasks,
            publication: tokio::sync::Mutex::new(()),
        })
    }

    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
    }

    pub(in crate::runtime::server) async fn reload(
        &self,
        path: &Path,
        runtime: &RuntimeCapabilities,
    ) -> Result<bool> {
        let _publication = self.publication.lock().await;
        self.check_running()?;
        let loaded = Loaded::read(path).await?;
        let current = self.active.load_full();
        if loaded.revision == current.revision {
            return Ok(false);
        }
        let revision = loaded.revision.clone();
        let bindings =
            Arc::new(Bindings::open(loaded, self.tasks.clone(), self.stopped.clone()).await?);
        // 上游校验期间可能收到下一次部署，禁止把已经过期的候选覆盖到目录。
        ensure!(
            Loaded::read(path).await?.revision == revision,
            "校验期间远端能力文件发生变化"
        );
        let candidate = Arc::new(Version {
            revision,
            generation: current
                .generation
                .checked_add(1)
                .context("远端能力代数溢出")?,
            bindings: bindings.clone(),
        });
        runtime.publish::<Self>(bindings, || {
            self.check_running()?;
            self.active.store(candidate);
            Ok(())
        })?;
        Ok(true)
    }

    fn check_running(&self) -> Result<()> {
        ensure!(
            !self.stopped.load(Ordering::Acquire) && !self.tasks.cancellation().is_cancelled(),
            "远端能力提供者已停止"
        );
        Ok(())
    }

    /// 仅观察部署方指定的固定文件；不向普通租户开放地址或凭据修改入口。
    pub fn watch(
        self: &Arc<Self>,
        path: std::path::PathBuf,
        runtime: &Arc<RuntimeCapabilities>,
    ) -> Result<()> {
        let provider = Arc::downgrade(self);
        let runtime = Arc::downgrade(runtime);
        let cancellation = self.tasks.cancellation();
        self.tasks.spawn(async move {
            let mut delay = Duration::from_secs(2);
            let mut reported = None;
            loop {
                tokio::select! {
                    _ = cancellation.cancelled() => return Ok(()),
                    _ = tokio::time::sleep(delay) => {},
                }
                let (Some(provider), Some(runtime)) = (provider.upgrade(), runtime.upgrade()) else {
                    return Ok(());
                };
                let result = tokio::select! {
                    _ = cancellation.cancelled() => return Ok(()),
                    result = tokio::time::timeout(Duration::from_secs(30), provider.reload(&path, &runtime)) => {
                        result.context("在线导入远端能力超时").and_then(|result| result)
                    },
                };
                match result {
                    Ok(changed) => {
                        delay = Duration::from_secs(2);
                        let recovered = reported.take().is_some();
                        if changed {
                            let active = provider.active.load();
                            eprintln!("远端能力已切换: generation={}, revision={}", active.generation, active.revision);
                        } else if recovered {
                            eprintln!("远端能力文件已恢复为当前活动版本");
                        }
                    }
                    Err(error) => {
                        let error = format!("{error:#}");
                        if reported.as_ref() != Some(&error) {
                            eprintln!("远端能力候选被拒绝，保留活动版本: {error}");
                            reported = Some(error);
                        }
                        delay = (delay * 2).min(Duration::from_secs(30));
                    }
                }
            }
        })
    }
}

#[async_trait::async_trait]
impl CapabilityProvider for RemoteCapabilities {
    fn capabilities(&self) -> Vec<RuntimeCapability> {
        self.active.load().bindings.capabilities()
    }

    fn snapshot(&self) -> Option<Arc<dyn CapabilityProvider>> {
        Some(self.active.load().bindings.clone())
    }

    fn available(&self, path: &str, tenant: &str) -> bool {
        self.active.load().bindings.available(path, tenant)
    }

    async fn invoke(&self, request: &CapabilityRequest) -> Result<Value> {
        let bindings = self.active.load().bindings.clone();
        bindings.invoke(request).await
    }
}
