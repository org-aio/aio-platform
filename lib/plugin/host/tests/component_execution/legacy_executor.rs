//! 替代执行器夹具验证宿主分发与生命周期；它不是生产 Wasm 解释器。

use std::{
    collections::HashSet,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use anyhow::{Context, Result, ensure};
use az_plugin_core::{Plugin, PluginFuture};
use az_plugin_host::runtime::{ComponentResponse, PageDefinition, server::plugins::*};

#[derive(Default)]
pub struct LegacyExecutor<const ID: u8> {
    pub validations: AtomicUsize,
    instances: Mutex<HashSet<(String, String, String)>>,
}

impl<const ID: u8> WasmArtifactValidator for LegacyExecutor<ID> {
    fn validate(&self, bytes: &[u8]) -> Result<Vec<PageDefinition>> {
        self.validations.fetch_add(1, Ordering::SeqCst);
        let bytes = bytes
            .strip_prefix(b"executor-fixture\n")
            .context("测试执行器拒绝该产物")?;
        Ok(serde_json::from_slice(bytes)?)
    }
}

impl<const ID: u8> WasmExecutor for LegacyExecutor<ID> {
    fn activate(
        &self,
        tenant: &str,
        source: &str,
        revision: &str,
        artifact: &Path,
    ) -> Result<WasmActivation> {
        let pages = self.validate(&std::fs::read(artifact)?)?;
        let created = self
            .instances
            .lock()
            .map_err(|_| anyhow::anyhow!("测试实例锁损坏"))?
            .insert((tenant.into(), source.into(), revision.into()));
        Ok(WasmActivation { pages, created })
    }
    fn deactivate(&self, tenant: &str, source: &str, revision: &str) -> Result<bool> {
        Ok(self
            .instances
            .lock()
            .map_err(|_| anyhow::anyhow!("测试实例锁损坏"))?
            .remove(&(tenant.into(), source.into(), revision.into())))
    }
    fn handle(
        &self,
        tenant: &str,
        source: &str,
        revision: &str,
        request: String,
    ) -> Result<ComponentResponse> {
        ensure!(
            self.instances
                .lock()
                .map_err(|_| anyhow::anyhow!("测试实例锁损坏"))?
                .contains(&(tenant.into(), source.into(), revision.into())),
            "测试实例未激活"
        );
        let request: serde_json::Value = serde_json::from_str(&request)?;
        ensure!(request["tenant_id"] == tenant, "调用上下文租户不一致");
        Ok(ComponentResponse {
            status: 200,
            content_type: "application/json".into(),
            body: serde_json::json!({"executor":ID,"revision":revision,"tenant":tenant})
                .to_string(),
        })
    }
    fn bind_revision(
        &self,
        tenant: &str,
        source: &str,
        backend: &str,
        revision: &str,
    ) -> Result<()> {
        let mut instances = self
            .instances
            .lock()
            .map_err(|_| anyhow::anyhow!("测试实例锁损坏"))?;
        ensure!(
            instances.contains(&(tenant.into(), source.into(), backend.into())),
            "测试后端不存在"
        );
        instances.insert((tenant.into(), source.into(), revision.into()));
        Ok(())
    }
    fn retain_development_revisions(
        &self,
        tenant: &str,
        source: &str,
        backend: &str,
        revision: &str,
    ) -> Result<()> {
        self.instances
            .lock()
            .map_err(|_| anyhow::anyhow!("测试实例锁损坏"))?
            .retain(|(t, s, r)| t != tenant || s != source || r == backend || r == revision);
        Ok(())
    }
    fn shutdown(&self) -> Result<()> {
        self.instances
            .lock()
            .map_err(|_| anyhow::anyhow!("测试实例锁损坏"))?
            .clear();
        Ok(())
    }
}

pub struct LegacyPlugin<const ID: u8>;

impl<const ID: u8> Plugin<RuntimeAssembly> for LegacyPlugin<ID> {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.declare_resource::<LegacyExecutor<ID>>()?;
            assembly
                .services
                .bind::<dyn WasmExecutor, LegacyExecutor<ID>>();
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    state
                        .service::<Resource<LegacyExecutor<ID>>>()?
                        .provide(Arc::new(LegacyExecutor::default()))
                })
            });
            assembly.on_stop::<Self>(|state, _| {
                Box::pin(async move {
                    if let Some(executor) = state
                        .service::<Resource<LegacyExecutor<ID>>>()?
                        .initialized()
                    {
                        executor.shutdown()?;
                    }
                    Ok(())
                })
            });
            Ok(())
        })
    }
}
