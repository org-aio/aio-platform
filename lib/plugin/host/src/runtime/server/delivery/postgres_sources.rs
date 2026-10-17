//! 默认来源与队列共用事务；独立发现插件不持有数据库连接。
use super::{DeliverySource, DeliverySources, DeliveryTarget};
use anyhow::{Result, ensure};
use sqlx::{PgPool, Row};
use std::sync::Arc;

#[dill::component]
#[dill::interface(dyn DeliverySources)]
#[dill::scope(dill::Singleton)]
pub(crate) struct PostgresDeliverySources {
    pool: Arc<PgPool>,
}
fn source(row: sqlx::postgres::PgRow) -> Result<DeliverySource> {
    Ok(DeliverySource {
        git: row.try_get("git")?,
        branch: row.try_get("branch")?,
        source_revision: row.try_get("desired_sha")?,
        enabled: row.try_get("enabled")?,
        generation: row.try_get("generation")?,
    })
}
#[async_trait::async_trait]
impl DeliverySources for PostgresDeliverySources {
    async fn get(&self, git: &str) -> Result<Option<DeliverySource>> {
        sqlx::query(
            "SELECT git,branch,desired_sha,enabled,generation FROM delivery_sources WHERE git=$1",
        )
        .bind(git)
        .fetch_optional(self.pool.as_ref())
        .await?
        .map(source)
        .transpose()
    }
    async fn enabled(&self) -> Result<Vec<DeliverySource>> {
        sqlx::query("SELECT git,branch,desired_sha,enabled,generation FROM delivery_sources WHERE enabled ORDER BY git")
            .fetch_all(self.pool.as_ref()).await?.into_iter().map(source).collect()
    }
    async fn update(
        &self,
        git: &str,
        expected: Option<i64>,
        target: Option<DeliveryTarget>,
    ) -> Result<bool> {
        ensure!(
            !git.is_empty() && git.len() <= 4096 && !git.contains('\0'),
            "交付仓库地址无效"
        );
        if let Some(target) = &target {
            target.validate()?;
        }
        let mut tx = self.pool.begin().await?;
        // 包括不存在的来源也需要串行化，随后仍按来源、任务的顺序锁定。
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,742613821))")
            .bind(git)
            .execute(&mut *tx)
            .await?;
        let generation: Option<i64> =
            sqlx::query_scalar("SELECT generation FROM delivery_sources WHERE git=$1 FOR UPDATE")
                .bind(git)
                .fetch_optional(&mut *tx)
                .await?;
        if generation != expected {
            return Ok(false);
        }
        let branch = target.as_ref().map(|t| t.branch.as_str()).unwrap_or("");
        let revision = target
            .as_ref()
            .map(|t| t.source_revision.as_str())
            .unwrap_or("");
        // 停用也保存墓碑，阻止先前读取到“不存在”的请求在撤销后入队。
        sqlx::query("INSERT INTO delivery_sources(git,branch,desired_sha,enabled,generation) VALUES($1,$2,$3,$4,1) ON CONFLICT(git) DO UPDATE SET branch=EXCLUDED.branch,desired_sha=EXCLUDED.desired_sha,enabled=EXCLUDED.enabled,generation=delivery_sources.generation+1,updated_at=now()")
            .bind(git).bind(branch).bind(revision).bind(target.is_some()).execute(&mut *tx).await?;
        sqlx::query("UPDATE delivery_jobs SET state='superseded',lease=NULL,lease_until=NULL,updated_at=now() WHERE git=$1 AND ($2='' OR source_revision<>$2) AND state IN ('queued','building','uploaded','publishing')")
            .bind(git).bind(revision).execute(&mut *tx).await?;
        if let Some(target) = target {
            let recipe = serde_json::to_value(target.recipe)?;
            sqlx::query("INSERT INTO delivery_jobs(git,source_revision,recipe) VALUES($1,$2,$3) ON CONFLICT(git,source_revision) DO UPDATE SET state='queued',lease=NULL,lease_until=NULL,error=NULL,retry_count=0,next_attempt_at=now(),updated_at=now() WHERE delivery_jobs.state='superseded'")
                .bind(git).bind(target.source_revision).bind(recipe).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(true)
    }
}
