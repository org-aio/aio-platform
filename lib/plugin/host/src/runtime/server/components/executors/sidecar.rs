use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, ensure};
use az_plugin_bundle::{Bundle, VerifiedBundle};
use az_plugin_contract::RequestContext;
use az_plugin_development::{DevArtifact, DevLaunch};
use uuid::Uuid;

use super::super::{
    execution::{ComponentDescription, ComponentExecutor, ExecutionContext, Request, Response},
    process::Processes,
};

/// 默认进程执行器；实例监督、broker 和开发进程资源沿用现有 Sidecar 实现。
#[derive(Default)]
pub struct SidecarExecutor {
    state: Mutex<Option<Arc<Processes>>>,
}

impl SidecarExecutor {
    fn state(&self) -> Result<Arc<Processes>> {
        self.state
            .lock()
            .map_err(|_| anyhow::anyhow!("Sidecar 执行器状态锁中毒"))?
            .clone()
            .context("Sidecar 执行器未初始化或已关闭")
    }
}

#[async_trait::async_trait]
impl ComponentExecutor for SidecarExecutor {
    fn supports(&self, bundle: &VerifiedBundle) -> bool {
        bundle.manifest().plugin.runtime.process.is_some()
    }

    async fn initialize(&self, context: ExecutionContext) -> Result<()> {
        let root = std::env::var_os("AIO_PROCESS_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| context.root.join("processes"));
        let supervisor = Processes::client()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("Sidecar 执行器状态锁中毒"))?;
        ensure!(state.is_none(), "Sidecar 执行器不能重复初始化");
        *state = Some(Arc::new(Processes::new(
            context.components,
            root,
            supervisor,
        )));
        Ok(())
    }

    async fn validate(&self, source: Uuid, bundle: &Bundle) -> Result<ComponentDescription> {
        let state = self.state()?;
        // 独立验证空间不会停止在线租户中的同版本进程。
        let (instance, description) = state
            .prepare(source, "publication-validation", bundle)
            .await?;
        let stopped = state.stop_id(&instance.start.id()).await;
        instance.resources.shutdown().await;
        stopped?;
        Ok(description)
    }

    async fn activate(&self, source: Uuid, tenant: &str, bundle: &Bundle) -> Result<()> {
        self.state()?.activate(source, tenant, bundle).await
    }

    async fn restore(&self, source: Uuid, tenant: &str, bundle: &Bundle) -> Result<()> {
        self.state()?.restore(source, tenant, bundle).await
    }

    async fn bundle(&self, source: Uuid, tenant: &str) -> Result<Option<Arc<VerifiedBundle>>> {
        Ok(self.state()?.bundle(source, tenant).await)
    }

    async fn handle(
        &self,
        source: Uuid,
        tenant: &str,
        digest: &str,
        request: Request,
        context: RequestContext,
    ) -> Result<Response> {
        self.state()?
            .handle(source, tenant, digest, request, context)
            .await
    }

    async fn deactivate(&self, source: Uuid, tenant: &str) -> Result<()> {
        if tenant == "development" {
            self.state()?.deactivate_development(source).await;
            return Ok(());
        }
        self.state()?.stop(source, tenant).await
    }

    async fn shutdown(&self) -> Result<()> {
        let state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("Sidecar 执行器状态锁中毒"))?
            .take();
        if let Some(state) = state {
            state.shutdown().await?;
        }
        Ok(())
    }

    async fn reconcile(&self) -> Result<()> {
        self.state()?.reconcile().await
    }

    async fn prepare_development(
        &self,
        artifact: &DevArtifact,
        bundle: Arc<VerifiedBundle>,
    ) -> Result<DevLaunch> {
        self.state()?.prepare_development(artifact, bundle).await
    }

    async fn activate_development(
        &self,
        artifact: &DevArtifact,
        bundle: Arc<VerifiedBundle>,
    ) -> Result<ComponentDescription> {
        self.state()?.activate_development(artifact, bundle).await
    }

    async fn discard_development_candidate(&self) -> Result<()> {
        let state = self.state()?;
        let pending = std::mem::take(&mut *state.pending.lock().await);
        for instance in pending.into_values() {
            instance.resources.shutdown().await;
        }
        Ok(())
    }
}
