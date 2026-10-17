//! 默认交付服务继续使用 PostgreSQL 跨表事务，接口层不依赖此实现。
use super::super::{RuntimeState, weak_state::WeakRuntimeState};
use super::{DeliveryArtifact, DeliveryService, PluginDocumentation, store};
use anyhow::Result;
use az_plugin_delivery::{BuildJob, BuildRecipe, BuildReport};
use az_plugin_package::PluginPackage;
use sqlx::{PgPool, Row};
use std::sync::Arc;

pub(crate) struct PostgresDelivery {
    pool: Arc<PgPool>,
    state: WeakRuntimeState,
}
impl PostgresDelivery {
    pub(crate) fn new(state: &RuntimeState) -> Result<Self> {
        Ok(Self {
            pool: state.service::<PgPool>()?,
            state: state.downgrade(),
        })
    }
}
#[async_trait::async_trait]
impl DeliveryService for PostgresDelivery {
    async fn claim(&self) -> Result<Option<BuildJob>> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(742613820)")
            .execute(&mut *tx)
            .await?;
        let busy: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM delivery_jobs WHERE state IN ('building','uploaded') AND lease_until>now())").fetch_one(&mut *tx).await?;
        if busy {
            return Ok(None);
        }
        let lease = uuid::Uuid::new_v4().to_string();
        let row = sqlx::query("WITH candidate AS (SELECT j.id FROM delivery_jobs j JOIN delivery_sources s ON s.git=j.git AND s.desired_sha=j.source_revision WHERE (j.state='queued' OR (j.state IN ('building','uploaded') AND j.lease_until < now())) AND j.next_attempt_at<=now() AND s.enabled ORDER BY j.next_attempt_at,j.id FOR UPDATE OF j SKIP LOCKED LIMIT 1) UPDATE delivery_jobs j SET state=CASE WHEN package_revision IS NULL THEN 'building' ELSE 'uploaded' END, lease=$1, lease_until=now()+interval '5 minutes', updated_at=now() FROM candidate WHERE j.id=candidate.id RETURNING j.id,j.git,j.source_revision,j.recipe")
        .bind(&lease).fetch_optional(&mut *tx).await?;
        tx.commit().await?;
        row.map(|row| {
            let id: i64 = row.try_get("id")?;
            let source_revision: String = row.try_get("source_revision")?;
            let recipe: BuildRecipe = serde_json::from_value(row.try_get("recipe")?)?;
            Ok(BuildJob {
                id,
                lease,
                git: row.try_get("git")?,
                version: format!("0.0.0-dev.{id}+{source_revision}"),
                source_revision,
                recipe,
            })
        })
        .transpose()
    }

    async fn retained_jobs(&self) -> Result<Vec<i64>> {
        Ok(sqlx::query_scalar("SELECT j.id FROM delivery_jobs j JOIN delivery_sources s ON s.git=j.git AND s.desired_sha=j.source_revision WHERE s.enabled AND j.state IN ('queued','building','uploaded','publishing')").fetch_all(self.pool.as_ref()).await?)
    }
    async fn defer(&self, id: i64, lease: &str, error: &str) -> Result<bool> {
        let updated = sqlx::query("UPDATE delivery_jobs j SET state='queued',error=$3,lease=NULL,lease_until=NULL,retry_count=retry_count+1,next_attempt_at=now()+make_interval(secs=>LEAST(300,30*power(2,LEAST(retry_count,4)))::double precision),updated_at=now() WHERE id=$1 AND lease=$2 AND lease_until>now() AND state IN ('building','uploaded') AND EXISTS(SELECT 1 FROM delivery_sources s WHERE s.git=j.git AND s.desired_sha=j.source_revision AND s.enabled)")
        .bind(id).bind(lease).bind(error.chars().take(16000).collect::<String>())
        .execute(self.pool.as_ref()).await?.rows_affected();
        Ok(updated == 1)
    }
    async fn heartbeat(&self, id: i64, lease: &str) -> Result<bool> {
        // 单语句复核当前目标、状态和租约，数据库故障不能伪装成租约冲突。
        let updated = sqlx::query("UPDATE delivery_jobs j SET lease_until=now()+interval '5 minutes',updated_at=now() WHERE id=$1 AND lease=$2 AND lease_until>now() AND state IN ('building','uploaded') AND EXISTS(SELECT 1 FROM delivery_sources s WHERE s.git=j.git AND s.desired_sha=j.source_revision AND s.enabled)")
        .bind(id).bind(lease).execute(self.pool.as_ref()).await?.rows_affected();
        Ok(updated == 1)
    }
    async fn upload(
        &self,
        id: i64,
        lease: &str,
        format: DeliveryArtifact,
        body: &[u8],
    ) -> Result<()> {
        let state = self.state.upgrade()?;
        let job = store::check_lease(self.pool.as_ref(), id, lease).await?;
        if matches!(format, DeliveryArtifact::Component) {
            super::components::upload(&state, &job, body).await?;
            return Ok(());
        }
        let body = body.to_vec();
        let package = tokio::task::spawn_blocking(move || PluginPackage::decode(&body)).await??;
        if package.git != job.git
            || package.source_revision.as_deref() != Some(&job.source_revision)
            || package.version != job.version
        {
            anyhow::bail!("构建产物与任务的仓库、源码提交或版本不符");
        }
        state.repository()?.stage_publish(&package).await?;
        let archive_package = package.clone();
        let archive = tokio::task::spawn_blocking(move || archive_package.encode()).await??;
        let mut tx = self.pool.begin().await?;
        store::lock_lease(&mut tx, &job).await?;
        super::super::package_store::save_package_archive(&mut tx, &package, &archive).await?;
        sqlx::query("UPDATE delivery_jobs SET package_revision=$3,state='uploaded',lease_until=now()+interval '5 minutes',updated_at=now() WHERE id=$1 AND lease=$2")
            .bind(id).bind(lease).bind(package.rev).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    async fn complete(&self, id: i64, report: BuildReport) -> Result<()> {
        let state = self.state.upgrade()?;
        let job = store::check_lease(self.pool.as_ref(), id, &report.lease).await?;
        if let Some(error) = report.error {
            let mut tx = self.pool.begin().await?;
            store::lock_lease(&mut tx, &job).await?;
            sqlx::query("UPDATE delivery_jobs SET state='failed',error=$3,lease_until=NULL,updated_at=now() WHERE id=$1 AND lease=$2")
                .bind(id).bind(report.lease).bind(error.chars().take(16000).collect::<String>()).execute(&mut *tx).await?;
            tx.commit().await?;
            return Ok(());
        }
        if super::components::complete(&state, &job, &report.documentation).await? {
            return Ok(());
        }
        let revision: Option<String> =
            sqlx::query_scalar("SELECT package_revision FROM delivery_jobs WHERE id=$1")
                .bind(id)
                .fetch_one(self.pool.as_ref())
                .await?;
        let revision = revision.ok_or_else(|| anyhow::anyhow!("尚未上传构建包"))?;
        super::documents::validate_documentation(&report.documentation)?;
        state
            .service::<dyn PluginDocumentation>()?
            .save(&revision, &report.documentation)
            .await?;
        state.restore_package_cache(&revision).await?;
        let staged = state
            .repository()?
            .validate_published(&job.git, &revision)
            .await?;
        let source_id = state
            .store()?
            .source_id(&job.git)
            .await?
            .unwrap_or(staged.source_id);
        let mut tx = self.pool.begin().await?;
        // 与目录激活保持同一加锁顺序：发布来源、交付来源、构建任务。
        super::super::publisher_store::lock_publication_source(&mut tx, "default", &job.git)
            .await?;
        store::lock_lease(&mut tx, &job).await?;
        let current: Option<String> =
            sqlx::query_scalar("SELECT package_revision FROM delivery_jobs WHERE id=$1")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        anyhow::ensure!(current.as_deref() == Some(&revision), "上传的构建包已变化");
        let publication = super::super::publisher_store::queue_publish_job(
            &mut tx,
            "default",
            &source_id,
            &job.git,
            &revision,
            staged.runtime,
            staged.pages.len(),
        )
        .await?;
        sqlx::query("UPDATE delivery_jobs SET state='publishing',lease_until=NULL,updated_at=now() WHERE id=$1 AND lease=$2")
            .bind(id).bind(report.lease).execute(&mut *tx).await?;
        tx.commit().await?;
        state.start_publish_job(publication)?;
        Ok(())
    }
    async fn retry(&self, id: i64) -> Result<bool> {
        let row = sqlx::query("UPDATE delivery_jobs j SET state='queued',error=NULL,lease=NULL,lease_until=NULL,retry_count=0,next_attempt_at=now(),updated_at=now() WHERE id=$1 AND state='failed' AND EXISTS(SELECT 1 FROM delivery_sources s WHERE s.git=j.git AND s.desired_sha=j.source_revision AND s.enabled) RETURNING id").bind(id).fetch_optional(self.pool.as_ref()).await?;
        Ok(row.is_some())
    }
    async fn builds(&self, git: &str) -> Result<Vec<serde_json::Value>> {
        sqlx::query("SELECT id,source_revision,state,error,updated_at::TEXT FROM delivery_jobs WHERE git=$1 ORDER BY id DESC LIMIT 10").bind(git).fetch_all(self.pool.as_ref()).await?
        .into_iter().map(|r| Ok(serde_json::json!({"id":r.try_get::<i64,_>("id")?,"source_revision":r.try_get::<String,_>("source_revision")?,"state":r.try_get::<String,_>("state")?,"error":r.try_get::<Option<String>,_>("error")?,"updated_at":r.try_get::<String,_>("updated_at")?}))).collect()
    }
}
