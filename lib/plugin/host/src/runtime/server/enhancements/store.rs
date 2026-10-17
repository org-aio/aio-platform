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
use sqlx::PgPool;
use tokio::sync::Semaphore;

use super::model::{Activation, Head, Revision};

/// 租户内增强版本的唯一持久化与执行入口；传输适配器不维护自己的版本指针。
pub(in crate::runtime::server) struct Enhancements {
    pool: PgPool,
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
        let prepared = self.load(tenant, &revision).await?;
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

    pub(in crate::runtime::server) async fn open(
        pool: PgPool,
        runtime: Arc<ScriptRuntime>,
        capabilities: Option<super::CapabilityEnvironment>,
    ) -> Result<Self> {
        sqlx::raw_sql(include_str!("schema.sql"))
            .execute(&pool)
            .await?;
        Ok(Self {
            pool,
            runtime,
            cache: Mutex::default(),
            execution: Arc::new(Semaphore::new(4)),
            capabilities,
        })
    }

    pub(in crate::runtime::server) async fn head(&self, tenant: &str) -> Result<Head> {
        Ok(sqlx::query_as(
            "SELECT revision,generation FROM runtime_enhancement_heads WHERE tenant_id=$1",
        )
        .bind(tenant)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or_default())
    }

    pub(in crate::runtime::server) async fn revisions(
        &self,
        tenant: &str,
    ) -> Result<Vec<Revision>> {
        Ok(sqlx::query_as("SELECT revision,author,(extract(epoch FROM created_at)*1000)::bigint AS created_at FROM runtime_enhancement_revisions WHERE tenant_id=$1 ORDER BY created_at DESC,revision LIMIT 50")
            .bind(tenant).fetch_all(&self.pool).await?)
    }

    /// 候选验证仅写不可变版本，不改变在线能力。
    pub(in crate::runtime::server) async fn propose(
        &self,
        tenant: &str,
        author: &str,
        package: Package,
        authorize: impl std::future::Future<Output = Result<()>>,
    ) -> Result<String> {
        let prepared = self.prepare(package).await?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query("INSERT INTO runtime_enhancement_revisions(tenant_id,revision,package,author) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING")
            .bind(tenant).bind(prepared.revision()).bind(serde_json::to_value(prepared.package())?).bind(author)
            .execute(&mut *transaction).await?;
        // 编译和数据库行锁等待都可能耗时；提交前重新核验，失败不保存候选。
        authorize.await?;
        transaction.commit().await?;
        self.remember(tenant, prepared.clone())?;
        Ok(prepared.revision().into())
    }

    pub(in crate::runtime::server) async fn package(
        &self,
        tenant: &str,
        revision: &str,
    ) -> Result<Package> {
        let value: serde_json::Value = sqlx::query_scalar(
            "SELECT package FROM runtime_enhancement_revisions WHERE tenant_id=$1 AND revision=$2",
        )
        .bind(tenant)
        .bind(revision)
        .fetch_optional(&self.pool)
        .await?
        .context("当前租户没有该增强版本")?;
        serde_json::from_value(value).context("读取增强包失败")
    }

    async fn load(&self, tenant: &str, revision: &str) -> Result<Arc<PreparedPackage>> {
        // 缓存按租户和摘要隔离，只有已持久验证的版本才能入缓存，命中时不重读源码。
        if let Some(prepared) = self
            .cache
            .lock()
            .map_err(|_| anyhow::anyhow!("增强缓存锁损坏"))?
            .get(&(tenant.to_owned(), revision.to_owned()))
            .cloned()
        {
            return Ok(prepared);
        }
        let package = self.package(tenant, revision).await?;
        let prepared = self.prepare(package).await?;
        ensure!(prepared.revision() == revision, "持久增强版本摘要不匹配");
        self.remember(tenant, prepared.clone())?;
        Ok(prepared)
    }

    /// 返回 None 表示活动代数已变化，调用方必须重新审阅后再发布。
    pub(in crate::runtime::server) async fn activate(
        &self,
        tenant: &str,
        actor: &str,
        request: Activation,
        rollback: bool,
        authorize: impl std::future::Future<Output = Result<()>>,
    ) -> Result<Option<Head>> {
        let revision = request.revision.as_deref();
        let expected_generation = request.expected_generation;
        ensure!(expected_generation >= 0, "活动代数无效");
        let previous = self.head(tenant).await?;
        if previous.generation != expected_generation {
            return Ok(None);
        }
        if rollback {
            let revision = revision.context("回滚必须指定曾激活的版本")?;
            let known: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runtime_enhancement_events WHERE tenant_id=$1 AND revision=$2)")
                .bind(tenant).bind(revision).fetch_one(&self.pool).await?;
            ensure!(known, "只能回滚到当前租户曾激活的版本");
        }
        let mut contracts = Vec::new();
        if let Some(revision) = revision {
            let candidate = self.load(tenant, revision).await?;
            contracts = candidate
                .package()
                .enhancements
                .iter()
                .map(|entry| entry.contract())
                .collect();
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
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO runtime_enhancement_heads(tenant_id) VALUES($1) ON CONFLICT DO NOTHING",
        )
        .bind(tenant)
        .execute(&mut *transaction)
        .await?;
        let next: Option<Head> = sqlx::query_as("UPDATE runtime_enhancement_heads SET revision=$2,generation=generation+1 WHERE tenant_id=$1 AND generation=$3 RETURNING revision,generation")
            .bind(tenant).bind(revision).bind(expected_generation).fetch_optional(&mut *transaction).await?;
        let Some(next) = next else {
            return Ok(None);
        };
        // CAS 已串行化当前租户发布；直接查询不可变历史，停用或回滚也不能释放已用契约。
        let conflict: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM runtime_enhancement_revisions r
                CROSS JOIN LATERAL jsonb_array_elements(r.package->'enhancements') old
                JOIN jsonb_array_elements($2::jsonb) candidate ON candidate->>'path'=old->>'path'
                WHERE r.tenant_id=$1
                  AND EXISTS(SELECT 1 FROM runtime_enhancement_events e WHERE e.tenant_id=r.tenant_id AND e.revision=r.revision)
                  AND (COALESCE(NULLIF(old->'input_schema','null'::jsonb),'true'::jsonb) <> candidate->'input_schema'
                    OR COALESCE(NULLIF(old->'output_schema','null'::jsonb),'true'::jsonb) <> candidate->'output_schema')
            )",
        ).bind(tenant).bind(serde_json::to_value(&contracts)?).fetch_one(&mut *transaction).await?;
        ensure!(
            !conflict,
            "增强路径的 schema 已发布，修改契约必须使用新版本路径"
        );
        let action = match (revision, rollback) {
            (None, _) => "deactivate",
            (_, true) => "rollback",
            _ => "activate",
        };
        sqlx::query("INSERT INTO runtime_enhancement_events(tenant_id,generation,revision,actor,action) VALUES($1,$2,$3,$4,$5)")
            .bind(tenant).bind(next.generation).bind(revision).bind(actor).bind(action).execute(&mut *transaction).await?;
        // 最后一次校验放在 CAS 和行锁等待之后；撤权或取消会回滚整个指针与审计事务。
        authorize.await?;
        transaction.commit().await?;
        Ok(Some(next))
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
        super::observations::started(&self.pool, tenant, &revision, head.generation, &path).await?;
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
        super::observations::finished(&self.pool, tenant, head.generation, &path, output.is_ok())
            .await
            .context("增强执行已结束，但观察计数保存失败")?;
        let output = output?;
        Ok(EnhancementResponse { revision, output })
    }

    pub(super) async fn history(&self, tenant: &str) -> Result<Vec<super::model::Event>> {
        Ok(sqlx::query_as("SELECT generation,revision,actor,action,(extract(epoch FROM created_at)*1000)::bigint AS created_at FROM runtime_enhancement_events WHERE tenant_id=$1 ORDER BY generation DESC LIMIT 50")
            .bind(tenant).fetch_all(&self.pool).await?)
    }

    pub(super) async fn observe(
        &self,
        tenant: &str,
        revision: &str,
        generation: i64,
    ) -> Result<Vec<super::model::Observation>> {
        // 先校验版本归属，不能用观察查询探测其他租户的摘要。
        let known: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runtime_enhancement_events WHERE tenant_id=$1 AND revision=$2 AND generation=$3)")
            .bind(tenant).bind(revision).bind(generation).fetch_one(&self.pool).await?;
        ensure!(known, "当前租户没有该版本与代数的激活记录");
        super::observations::list(&self.pool, tenant, generation).await
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
