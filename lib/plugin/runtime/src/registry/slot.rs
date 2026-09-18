use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use az_plugin_bundle::Bundle;
use az_plugin_contract::{CapabilityGrants, RequestContext};
use sqlx::PgPool;
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::{
    ComponentEngine, ComponentSlot, DatabaseProvisioner, InvocationResources, ReleaseSnapshot,
    bindings::aio::plugin::transport::{Request, Response},
};

use super::{StoredRelease, storage};

pub struct PersistentComponentSlot {
    pool: PgPool,
    source: Uuid,
    tenant: String,
    host_version: semver::Version,
    updates: Mutex<()>,
    state: RwLock<Option<Arc<ComponentSlot>>>,
}

impl DatabaseProvisioner {
    pub async fn component_slot(
        &self,
        source: Uuid,
        tenant: String,
        host_version: semver::Version,
    ) -> Result<PersistentComponentSlot> {
        ensure!(
            !source.is_nil() && !tenant.is_empty() && tenant.len() <= 128,
            "安装归属无效"
        );
        storage::initialize(&self.pool).await?;
        Ok(PersistentComponentSlot {
            pool: self.pool.clone(),
            source,
            tenant,
            host_version,
            updates: Mutex::new(()),
            state: RwLock::new(None),
        })
    }
}

impl PersistentComponentSlot {
    pub async fn stored(&self) -> Result<Option<StoredRelease>> {
        storage::stored(&self.pool, self.source, &self.tenant).await
    }

    pub async fn activate(
        &self,
        engine: &ComponentEngine,
        bundle: Bundle,
        grants: CapabilityGrants,
        resources: InvocationResources,
    ) -> Result<()> {
        let _update = self.updates.lock().await;
        let expected = storage::generation(&self.pool, self.source, &self.tenant).await?;
        if let Some(previous) = self.stored().await? {
            storage::validate_upgrade(&previous.bundle, &bundle)?;
        }
        let archive = bundle.encode()?;
        let candidate = self
            .prepare(engine, bundle.clone(), grants.clone(), resources)
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
        storage::activate(
            &self.pool,
            self.source,
            &self.tenant,
            expected,
            &bundle.digest,
            &archive,
            &actual_grants,
        )
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
        let mut transaction = self.pool.begin().await?;
        let (revision, generation) =
            storage::locked_revision(&mut transaction, self.source, &self.tenant, false).await?;
        ensure!(
            revision.as_deref() == Some(&stored.bundle.digest) && generation == stored.generation,
            "恢复期间活动版本已变更"
        );
        let previous = state.replace(candidate);
        transaction.commit().await?;
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
        let mut transaction = self.pool.begin().await?;
        let (revision, _) =
            storage::locked_revision(&mut transaction, self.source, &self.tenant, false).await?;
        ensure!(
            revision.as_deref() == Some(snapshot.bundle.digest()),
            "活动版本已改变，需要恢复执行槽"
        );
        transaction.commit().await?;
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
        let mut transaction = self.pool.begin().await?;
        let (revision, _) =
            storage::locked_revision(&mut transaction, self.source, &self.tenant, false).await?;
        ensure!(
            revision.as_deref() == Some(digest),
            "页面版本已撤销，请重新挂载"
        );
        let response = match active.handle(digest, request, context).await {
            Ok(response) => response,
            Err(error) => {
                drop(state);
                self.invalidate().await;
                return Err(error);
            }
        };
        transaction.commit().await?;
        Ok(response)
    }

    pub async fn deactivate(&self) -> Result<()> {
        let _update = self.updates.lock().await;
        let mut state = self.state.write().await;
        storage::deactivate(&self.pool, self.source, &self.tenant).await?;
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

    async fn invalidate(&self) {
        let _update = self.updates.lock().await;
        *self.state.write().await = None;
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
