use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, ensure};
use az_plugin_bundle::{Bundle, VerifiedBundle};
use az_plugin_contract::RequestContext;
use az_plugin_development::DevArtifact;
use az_plugin_runtime::{ComponentEngine, ComponentSlot, PersistentComponentSlot};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

use super::super::execution::{
    ComponentDescription, ComponentExecutor, ExecutionContext, Request, Response,
};

/// 默认 Wasm Component 执行器；只有被选择并初始化时才创建引擎和编译缓存。
#[derive(Default)]
pub struct WasmComponentExecutor {
    state: Mutex<Option<Arc<State>>>,
}

struct State {
    context: ExecutionContext,
    engine: ComponentEngine,
    slots: AsyncMutex<HashMap<(Uuid, String), Arc<PersistentComponentSlot>>>,
    development: AsyncMutex<HashMap<Uuid, Arc<ComponentSlot>>>,
}

impl WasmComponentExecutor {
    fn state(&self) -> Result<Arc<State>> {
        self.state
            .lock()
            .map_err(|_| anyhow::anyhow!("Wasm 执行器状态锁中毒"))?
            .clone()
            .context("Wasm 执行器未初始化或已关闭")
    }

    async fn slot(&self, source: Uuid, tenant: &str) -> Result<Arc<PersistentComponentSlot>> {
        let state = self.state()?;
        let mut slots = state.slots.lock().await;
        let key = (source, tenant.to_owned());
        if let Some(slot) = slots.get(&key) {
            return Ok(slot.clone());
        }
        let components = state.context.components()?;
        let slot = Arc::new(
            components
                .provisioner
                .component_slot(
                    source,
                    tenant.into(),
                    semver::Version::parse(env!("CARGO_PKG_VERSION"))?,
                )
                .await?,
        );
        slots.insert(key, slot.clone());
        Ok(slot)
    }

    async fn restored_slot(
        &self,
        source: Uuid,
        tenant: &str,
    ) -> Result<Arc<PersistentComponentSlot>> {
        let state = self.state()?;
        let slot = self.slot(source, tenant).await?;
        if slot.snapshot().await?.is_none()
            && let Some(stored) = slot.stored().await?
        {
            let resources = state
                .context
                .resources(source, tenant, &stored.bundle.verify()?)
                .await?;
            slot.restore(&state.engine, stored.grants, resources)
                .await?;
        }
        Ok(slot)
    }
}

#[async_trait::async_trait]
impl ComponentExecutor for WasmComponentExecutor {
    fn supports(&self, bundle: &VerifiedBundle) -> bool {
        bundle.manifest().plugin.runtime.process.is_none()
    }

    async fn initialize(&self, context: ExecutionContext) -> Result<()> {
        let engine = ComponentEngine::with_cache(&context.root.join("compiled"))?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("Wasm 执行器状态锁中毒"))?;
        ensure!(state.is_none(), "Wasm 执行器不能重复初始化");
        *state = Some(Arc::new(State {
            context,
            engine,
            slots: Default::default(),
            development: Default::default(),
        }));
        Ok(())
    }

    async fn validate(&self, source: Uuid, bundle: &Bundle) -> Result<ComponentDescription> {
        let state = self.state()?;
        let verified = Arc::new(bundle.verify()?);
        let resources = state
            .context
            .resources(source, "default", &verified)
            .await?;
        let slot = ComponentSlot::new(
            source,
            "default".into(),
            semver::Version::parse(env!("CARGO_PKG_VERSION"))?,
        )?;
        slot.replace(
            &state.engine,
            verified.clone(),
            verified.manifest().plugin.capabilities.clone(),
            resources,
        )
        .await?;
        let description = slot.snapshot().await.context("候选实例未激活")?.description;
        slot.deactivate().await;
        ComponentDescription::from(description).with_settings(&verified)
    }

    async fn activate(&self, source: Uuid, tenant: &str, bundle: &Bundle) -> Result<()> {
        let state = self.state()?;
        let verified = bundle.verify()?;
        let resources = state.context.resources(source, tenant, &verified).await?;
        self.slot(source, tenant)
            .await?
            .activate(
                &state.engine,
                bundle.clone(),
                verified.manifest().plugin.capabilities.clone(),
                resources,
            )
            .await
    }

    async fn restore(&self, source: Uuid, tenant: &str, bundle: &Bundle) -> Result<()> {
        let slot = self.slot(source, tenant).await?;
        if slot
            .snapshot()
            .await?
            .is_some_and(|snapshot| snapshot.bundle.digest() == bundle.digest)
        {
            return Ok(());
        }
        // 安装记录是提交点；重启时修复执行槽中未完成提交的候选版本。
        self.activate(source, tenant, bundle).await
    }

    async fn bundle(&self, source: Uuid, tenant: &str) -> Result<Option<Arc<VerifiedBundle>>> {
        let state = self.state()?;
        if tenant == "development" {
            let slot = state.development.lock().await.get(&source).cloned();
            return match slot {
                Some(slot) => Ok(slot.snapshot().await.map(|snapshot| snapshot.bundle)),
                None => Ok(None),
            };
        }
        Ok(self
            .restored_slot(source, tenant)
            .await?
            .snapshot()
            .await?
            .map(|snapshot| snapshot.bundle))
    }

    async fn handle(
        &self,
        source: Uuid,
        tenant: &str,
        digest: &str,
        request: Request,
        context: RequestContext,
    ) -> Result<Response> {
        let state = self.state()?;
        if tenant == "development" {
            let slot = state
                .development
                .lock()
                .await
                .get(&source)
                .cloned()
                .context("开发实例未激活")?;
            return slot.handle(digest, request, context).await;
        }
        let slot = self.restored_slot(source, tenant).await?;
        let retry = (request.clone(), context.clone());
        match slot.handle(digest, request, context).await {
            Ok(response) => Ok(response),
            Err(error) => {
                slot.unload().await;
                if format!("{error:#}").contains("实例已失效，必须重新创建") {
                    // 仅执行前的失效可重试；写入超时或取消的结果不确定，不能重放。
                    let slot = self.restored_slot(source, tenant).await?;
                    return slot.handle(digest, retry.0, retry.1).await;
                }
                Err(error)
            }
        }
    }

    async fn deactivate(&self, source: Uuid, tenant: &str) -> Result<()> {
        let state = self.state()?;
        if tenant == "development" {
            if let Some(slot) = state.development.lock().await.remove(&source) {
                slot.deactivate().await;
            }
            return Ok(());
        }
        self.slot(source, tenant).await?.deactivate().await
    }

    async fn shutdown(&self) -> Result<()> {
        let state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("Wasm 执行器状态锁中毒"))?
            .take();
        let Some(state) = state else {
            return Ok(());
        };
        let slots = std::mem::take(&mut *state.slots.lock().await);
        for slot in slots.into_values() {
            slot.unload().await;
        }
        let development = std::mem::take(&mut *state.development.lock().await);
        for slot in development.into_values() {
            slot.deactivate().await;
        }
        Ok(())
    }

    async fn activate_development(
        &self,
        artifact: &DevArtifact,
        bundle: Arc<VerifiedBundle>,
    ) -> Result<ComponentDescription> {
        let state = self.state()?;
        let source = Uuid::new_v5(&Uuid::NAMESPACE_URL, artifact.source.as_bytes());
        let previous = state.development.lock().await.get(&source).cloned();
        let slot = match previous {
            Some(slot) => slot,
            None => Arc::new(ComponentSlot::new(
                source,
                "development".into(),
                semver::Version::parse(env!("CARGO_PKG_VERSION"))?,
            )?),
        };
        if !slot.refresh_frontend(bundle.clone()).await? {
            let resources = state
                .context
                .resources(source, "development", &bundle)
                .await?;
            slot.replace(
                &state.engine,
                bundle.clone(),
                bundle.manifest().plugin.capabilities.clone(),
                resources,
            )
            .await?;
        }
        let description = ComponentDescription::from(
            slot.snapshot().await.context("开发实例未激活")?.description,
        )
        .with_settings(&bundle)?;
        state.development.lock().await.insert(source, slot);
        Ok(description)
    }
}
