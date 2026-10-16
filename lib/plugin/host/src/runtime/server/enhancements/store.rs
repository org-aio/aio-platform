use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, ensure};
use az_plugin_contract::{EnhancementRequest, EnhancementResponse, InvocationScope};
use az_plugin_enhancement::{
    Package, PreparedPackage, ScriptContext, ScriptInvocation, ScriptRuntime,
};
use serde::Serialize;
use sqlx::PgPool;
use tokio::sync::Semaphore;

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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub(in crate::runtime::server) struct Head {
    pub revision: Option<String>,
    pub generation: i64,
}

#[derive(Serialize, sqlx::FromRow)]
pub(in crate::runtime::server) struct Revision {
    pub revision: String,
    pub author: String,
    pub created_at: i64,
}

impl Enhancements {
    pub(in crate::runtime::server) fn runtime_catalog(
        &self,
        tenant: &str,
    ) -> Result<Vec<super::RuntimeCapability>> {
        self.capabilities
            .as_ref()
            .map(|environment| environment.runtime()?.catalog(tenant))
            .unwrap_or_else(|| Ok(vec![]))
    }

    /// 发现和调用读取同一活动指针；目录不暴露源码，也不保存独立的协议版本状态。
    pub(in crate::runtime::server) async fn catalog(
        &self,
        tenant: &str,
        permitted: impl Fn(&str) -> bool,
        permitted_capability: impl Fn(&str) -> bool,
    ) -> Result<(Option<String>, Vec<String>)> {
        let Some(revision) = self.head(tenant).await?.revision else {
            return Ok((None, Vec::new()));
        };
        let prepared = self.load(tenant, &revision).await?;
        let available = self.runtime_catalog(tenant)?;
        let mut paths = Vec::new();
        for item in &prepared.package().enhancements {
            let required = prepared.required_paths(&item.path)?;
            let capabilities = prepared.required_capabilities(&item.path)?;
            if required.iter().all(|path| permitted(path))
                && capabilities.iter().all(|path| {
                    permitted_capability(path) && available.iter().any(|entry| &entry.path == path)
                })
            {
                paths.push(item.path.clone());
            }
        }
        paths.sort();
        Ok((Some(revision), paths))
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
    ) -> Result<String> {
        let prepared = self.prepare(package).await?;
        sqlx::query("INSERT INTO runtime_enhancement_revisions(tenant_id,revision,package,author) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING")
            .bind(tenant).bind(prepared.revision()).bind(serde_json::to_value(prepared.package())?).bind(author)
            .execute(&self.pool).await?;
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
        revision: Option<&str>,
        expected_generation: i64,
        rollback: bool,
    ) -> Result<Option<Head>> {
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
        let action = match (revision, rollback) {
            (None, _) => "deactivate",
            (_, true) => "rollback",
            _ => "activate",
        };
        sqlx::query("INSERT INTO runtime_enhancement_events(tenant_id,generation,revision,actor,action) VALUES($1,$2,$3,$4,$5)")
            .bind(tenant).bind(next.generation).bind(revision).bind(actor).bind(action).execute(&mut *transaction).await?;
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
        let revision = self
            .head(tenant)
            .await?
            .revision
            .context("当前租户未激活增强")?;
        let prepared = self.load(tenant, &revision).await?;
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
        .context("等待增强执行失败")??;
        Ok(EnhancementResponse { revision, output })
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
