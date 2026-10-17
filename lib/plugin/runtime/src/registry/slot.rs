use az_plugin_contract::transport::{Request, Response};
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use az_plugin_bundle::Bundle;
use az_plugin_contract::{CapabilityGrants, RequestContext};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::{
    ComponentEngine, ComponentReleases, ComponentSlot, InvocationResources, ReleaseSnapshot,
    StoredRelease,
};

pub struct PersistentComponentSlot {
    releases: Arc<dyn ComponentReleases>,
    source: Uuid,
    tenant: String,
    host_version: semver::Version,
    updates: Mutex<()>,
    state: RwLock<Option<Arc<ComponentSlot>>>,
}

impl PersistentComponentSlot {
    /// 绑定发布记录契约；执行器不依赖具体数据库或锁实现。
    pub async fn open(
        releases: Arc<dyn ComponentReleases>,
        source: Uuid,
        tenant: String,
        host_version: semver::Version,
    ) -> Result<PersistentComponentSlot> {
        ensure!(
            !source.is_nil() && !tenant.is_empty() && tenant.len() <= 128,
            "安装归属无效"
        );
        releases.initialize().await?;
        Ok(Self {
            releases,
            source,
            tenant,
            host_version,
            updates: Mutex::new(()),
            state: RwLock::new(None),
        })
    }
    pub async fn stored(&self) -> Result<Option<StoredRelease>> {
        self.releases.stored(self.source, &self.tenant).await
    }

    pub async fn activate(
        &self,
        engine: &ComponentEngine,
        bundle: Bundle,
        grants: CapabilityGrants,
        resources: InvocationResources,
    ) -> Result<()> {
        let _update = self.updates.lock().await;
        let expected = self.releases.generation(self.source, &self.tenant).await?;
        if let Some(previous) = self.stored().await? {
            let old = previous.bundle.verify()?;
            let next = bundle.verify()?;
            ensure!(
                next.migrations()
                    .collect::<Vec<_>>()
                    .starts_with(&old.migrations().collect::<Vec<_>>()),
                "升级不能改写或删除已应用的迁移"
            );
        }
        let candidate = self
            .prepare(engine, bundle.clone(), grants, resources)
            .await?;
        let actual_grants = candidate
            .snapshot()
            .await
            .context("候选实例不可用")?
            .bundle
            .manifest()
            .plugin
            .capabilities
            .clone();
        let mut state = self.state.write().await;
        self.releases
            .activate(self.source, &self.tenant, expected, &bundle, &actual_grants)
            .await?;
        // 提交后的本地指针交换不包含等待点；提交结果不确定时 handle 会校验数据库版本。
        let previous = state.replace(candidate);
        drop(state);
        if let Some(previous) = previous {
            previous.deactivate().await;
        }
        Ok(())
    }

    pub async fn restore(
        &self,
        engine: &ComponentEngine,
        grants: CapabilityGrants,
        resources: InvocationResources,
    ) -> Result<bool> {
        let _update = self.updates.lock().await;
        let Some(stored) = self.stored().await? else {
            return Ok(false);
        };
        let candidate = self
            .prepare(engine, stored.bundle.clone(), grants, resources)
            .await?;
        let mut state = self.state.write().await;
        let lease = self.releases.lease(self.source, &self.tenant).await?;
        let version = lease.version();
        ensure!(
            version.revision.as_deref() == Some(&stored.bundle.digest)
                && version.generation == stored.generation,
            "恢复期间活动版本已变更"
        );
        lease.finish().await?;
        let previous = state.replace(candidate);
        drop(state);
        if let Some(previous) = previous {
            previous.deactivate().await;
        }
        Ok(true)
    }

    pub async fn snapshot(&self) -> Result<Option<ReleaseSnapshot>> {
        let state = self.state.read().await;
        let Some(active) = state.as_ref() else {
            return Ok(None);
        };
        let snapshot = active.snapshot().await.context("活动实例不可用")?;
        let lease = self.releases.lease(self.source, &self.tenant).await?;
        let revision = &lease.version().revision;
        ensure!(
            revision.as_deref() == Some(snapshot.bundle.digest()),
            "活动版本已改变，需要恢复执行槽"
        );
        lease.finish().await?;
        Ok(Some(snapshot))
    }

    pub async fn handle(
        &self,
        digest: &str,
        request: Request,
        context: RequestContext,
    ) -> Result<Response> {
        ensure!(
            context.tenant_id.as_deref() == Some(&self.tenant),
            "调用租户不匹配"
        );
        let state = self.state.read().await;
        let active = state.as_ref().context("插件未激活")?.clone();
        let lease = self.releases.lease(self.source, &self.tenant).await?;
        let revision = &lease.version().revision;
        ensure!(
            revision.as_deref() == Some(digest),
            "页面版本已撤销，请重新挂载"
        );
        let response = match active.handle(digest, request, context).await {
            Ok(response) => response,
            Err(error) => {
                // 先释放跨宿主读租约，避免与持有更新锁的并发激活互相等待。
                let released = lease.finish().await;
                drop(state);
                self.invalidate(&active).await;
                if let Err(release_error) = released {
                    return Err(error.context(format!("释放发布读租约失败: {release_error:#}")));
                }
                return Err(error);
            }
        };
        lease.finish().await?;
        Ok(response)
    }

    pub async fn deactivate(&self) -> Result<()> {
        let _update = self.updates.lock().await;
        let mut state = self.state.write().await;
        self.releases.deactivate(self.source, &self.tenant).await?;
        let previous = state.take();
        drop(state);
        if let Some(previous) = previous {
            previous.deactivate().await;
        }
        Ok(())
    }

    pub async fn unload(&self) {
        let _update = self.updates.lock().await;
        let previous = self.state.write().await.take();
        if let Some(previous) = previous {
            previous.deactivate().await;
        }
    }

    async fn invalidate(&self, failed: &Arc<ComponentSlot>) {
        let _update = self.updates.lock().await;
        let mut state = self.state.write().await;
        if state
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, failed))
        {
            *state = None;
        }
    }

    async fn prepare(
        &self,
        engine: &ComponentEngine,
        bundle: Bundle,
        grants: CapabilityGrants,
        resources: InvocationResources,
    ) -> Result<Arc<ComponentSlot>> {
        let candidate = Arc::new(ComponentSlot::new(
            self.source,
            self.tenant.clone(),
            self.host_version.clone(),
        )?);
        candidate
            .replace(engine, Arc::new(bundle.verify()?), grants, resources)
            .await?;
        Ok(candidate)
    }
}
