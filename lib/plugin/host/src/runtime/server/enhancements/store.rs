#[cfg(any(feature = "mcp", feature = "grpc"))]
use az_plugin_enhancement::EnhancementContract;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, ensure};
use az_plugin_contract::{EnhancementRequest, EnhancementResponse, InvocationScope};
use az_plugin_enhancement::{
    Package, PreparedPackage, ScriptContext, ScriptInvocation, ScriptRuntime,
};
use tokio::sync::Semaphore;

use super::super::enhancement_catalog::{
    Activation, EnhancementCatalog, Event, Head, Observation, Revision,
};

/// 租户内增强版本的唯一持久化与执行入口；传输适配器不维护自己的版本指针。
pub(in crate::runtime::server) struct Enhancements {
    catalog: Arc<dyn EnhancementCatalog>,
    runtime: Arc<ScriptRuntime>,
    cache: Mutex<BTreeMap<(String, String), Arc<PreparedPackage>>>,
    execution: Arc<Semaphore>,
    capabilities: Option<super::CapabilityEnvironment>,
}

// 请求等待被取消时通知整条脚本调用链；并发配额仍由工作线程持有到真正退出。
struct CancelInvocation(ScriptInvocation);

impl Drop for CancelInvocation {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

impl Enhancements {
    pub(in crate::runtime::server) fn runtime_catalog(
        &self,
        tenant: &str,
    ) -> Result<Vec<super::super::capabilities::RuntimeCapability>> {
        self.capabilities
            .as_ref()
            .map(|environment| environment.runtime()?.catalog(tenant))
            .unwrap_or_else(|| Ok(vec![]))
    }

    /// 发现和调用读取同一活动指针；目录不暴露源码，也不保存独立的协议版本状态。
    #[cfg(any(feature = "mcp", feature = "grpc"))]
    pub(in crate::runtime::server) async fn catalog(
        &self,
        tenant: &str,
        permitted: impl Fn(&str) -> bool,
        permitted_capability: impl Fn(&str) -> bool,
    ) -> Result<(Option<String>, Vec<EnhancementContract>)> {
        let Some(revision) = self.head(tenant).await?.revision else {
            return Ok((None, Vec::new()));
        };
        let Some(prepared) = self.load_available(tenant, &revision).await? else {
            return Ok((Some(revision), Vec::new()));
        };
        let available = self.runtime_catalog(tenant)?;
        let mut contracts = Vec::new();
        for item in &prepared.package().enhancements {
            let required = prepared.required_paths(&item.path)?;
            let capabilities = prepared.required_capabilities(&item.path)?;
            if required.iter().all(|path| permitted(path))
                && capabilities.iter().all(|path| {
                    permitted_capability(path) && available.iter().any(|entry| &entry.path == path)
                })
            {
                contracts.push(item.contract());
            }
        }
        contracts.sort_by(|left, right| left.path.cmp(&right.path));
        Ok((Some(revision), contracts))
    }

    pub(in crate::runtime::server) fn new(
        catalog: Arc<dyn EnhancementCatalog>,
        runtime: Arc<ScriptRuntime>,
        capabilities: Option<super::CapabilityEnvironment>,
    ) -> Self {
        Self {
            catalog,
            runtime,
            cache: Mutex::default(),
            execution: Arc::new(Semaphore::new(4)),
            capabilities,
        }
    }

    pub(in crate::runtime::server) async fn head(&self, tenant: &str) -> Result<Head> {
        self.catalog.head(tenant).await
    }

    pub(in crate::runtime::server) async fn revisions(
        &self,
        tenant: &str,
    ) -> Result<Vec<Revision>> {
        self.catalog.revisions(tenant).await
    }

    /// 候选验证仅写不可变版本，不改变在线能力。
    pub(in crate::runtime::server) async fn propose(
        &self,
        tenant: &str,
        author: &str,
        package: Package,
        authorize: impl std::future::Future<Output = Result<()>> + Send,
    ) -> Result<String> {
        let prepared = self.prepare(package).await?;
        self.catalog
            .propose(tenant, author, prepared.package(), Box::pin(authorize))
            .await?;
        self.remember(tenant, prepared.clone())?;
        Ok(prepared.revision().into())
    }

    pub(in crate::runtime::server) async fn package(
        &self,
        tenant: &str,
        revision: &str,
    ) -> Result<Package> {
        self.catalog.package(tenant, revision).await
    }

    async fn load(&self, tenant: &str, revision: &str) -> Result<Arc<PreparedPackage>> {
        self.load_available(tenant, revision)
            .await?
            .context("未装配增强包所需的脚本执行器")
    }

    // 整包仍是编译执行单元；缺失语言只暂停发现，持久化或编译错误不得被吞掉。
    async fn load_available(
        &self,
        tenant: &str,
        revision: &str,
    ) -> Result<Option<Arc<PreparedPackage>>> {
        // 缓存按租户和摘要隔离，只有已持久验证的版本才能入缓存，命中时不重读源码。
        if let Some(prepared) = self
            .cache
            .lock()
            .map_err(|_| anyhow::anyhow!("增强缓存锁损坏"))?
            .get(&(tenant.to_owned(), revision.to_owned()))
            .cloned()
        {
            return Ok(Some(prepared));
        }
        let package = self.package(tenant, revision).await?;
        ensure!(package.revision()? == revision, "持久增强版本摘要不匹配");
        let mut available = true;
        for enhancement in &package.enhancements {
            available &= self.runtime.supports(enhancement.language)?;
        }
        if !available {
            return Ok(None);
        }
        let prepared = self.prepare(package).await?;
        self.remember(tenant, prepared.clone())?;
        Ok(Some(prepared))
    }

    /// 返回 None 表示活动代数已变化，调用方必须重新审阅后再发布。
    pub(in crate::runtime::server) async fn activate(
        &self,
        tenant: &str,
        actor: &str,
        request: Activation,
        rollback: bool,
        authorize: impl std::future::Future<Output = Result<()>> + Send,
    ) -> Result<Option<Head>> {
        let revision = request.revision.as_deref();
        let expected_generation = request.expected_generation;
        ensure!(expected_generation >= 0, "活动代数无效");
        let previous = self.head(tenant).await?;
        if previous.generation != expected_generation {
            return Ok(None);
        }
        if let Some(revision) = revision {
            let candidate = self.load(tenant, revision).await?;
            let available = self.runtime_catalog(tenant)?;
            for entry in &candidate.package().enhancements {
                for path in &entry.capabilities {
                    ensure!(
                        available.iter().any(|capability| &capability.path == path),
                        "增强依赖的宿主能力未装配: {path}"
                    );
                }
            }
            if !rollback && let Some(previous) = previous.revision {
                let package = self.package(tenant, &previous).await?;
                let permit = self
                    .execution
                    .clone()
                    .try_acquire_owned()
                    .context("增强验证并发已满")?;
                tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    candidate.check_examples(&package)
                })
                .await
                .context("等待增强契约验证失败")??;
            }
        }
        self.catalog
            .activate(tenant, actor, &request, rollback, Box::pin(authorize))
            .await
    }

    pub(in crate::runtime::server) async fn invoke(
        &self,
        scope: &InvocationScope,
        request: EnhancementRequest,
        deadline: Option<std::time::Instant>,
    ) -> Result<EnhancementResponse> {
        ensure!(
            scope.grants.enhancements.contains(&request.path),
            "插件未获该增强能力"
        );
        let tenant = scope
            .context
            .tenant_id
            .as_deref()
            .filter(|value| !value.is_empty())
            .context("增强调用缺少租户")?;
        let head = self.head(tenant).await?;
        let revision = head.revision.context("当前租户未激活增强")?;
        let prepared = self.load(tenant, &revision).await?;
        prepared.required_paths(&request.path)?;
        let context = ScriptContext {
            tenant_id: tenant.into(),
            source_id: scope.source_id.clone(),
        };
        let permit = self
            .execution
            .clone()
            .try_acquire_owned()
            .context("增强执行并发已满")?;
        let capabilities = self
            .capabilities
            .as_ref()
            .map(|environment| environment.bind(scope))
            .transpose()?
            .unwrap_or_default();
        let path = request.path.clone();
        self.catalog
            .started(tenant, &revision, head.generation, &path)
            .await?;
        let invocation = prepared.invocation(
            context,
            scope.grants.enhancements.clone(),
            capabilities,
            deadline,
        );
        let _cancel = CancelInvocation(invocation.clone());
        // 将配额交给工作线程；取消 HTTP/WIT 等待不会提早释放仍在执行的配额。
        let output = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            invocation.invoke(&request.path, &request.input)
        })
        .await
        .context("等待增强执行失败")
        .and_then(|output| output);
        // 协议对外隐藏内部错误；测试失败时保留根因，且不输出请求数据。
        #[cfg(test)]
        if let Err(error) = &output {
            eprintln!("增强测试调用错误: {error:#}");
        }
        self.catalog
            .finished(tenant, head.generation, &path, output.is_ok())
            .await
            .context("增强执行已结束，但观察计数保存失败")?;
        let output = output?;
        Ok(EnhancementResponse { revision, output })
    }

    pub(super) async fn history(&self, tenant: &str) -> Result<Vec<Event>> {
        self.catalog.history(tenant).await
    }

    pub(super) async fn observe(
        &self,
        tenant: &str,
        revision: &str,
        generation: i64,
    ) -> Result<Vec<Observation>> {
        self.catalog.observe(tenant, revision, generation).await
    }

    async fn prepare(&self, package: Package) -> Result<Arc<PreparedPackage>> {
        let runtime = self.runtime.clone();
        let permit = self
            .execution
            .clone()
            .try_acquire_owned()
            .context("增强验证并发已满")?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            runtime.prepare(package).map(Arc::new)
        })
        .await
        .context("等待增强编译失败")?
    }

    fn remember(&self, tenant: &str, prepared: Arc<PreparedPackage>) -> Result<()> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| anyhow::anyhow!("增强缓存锁损坏"))?;
        if cache.len() >= 16 {
            cache.clear();
        }
        cache.insert((tenant.into(), prepared.revision().into()), prepared);
        Ok(())
    }
}
