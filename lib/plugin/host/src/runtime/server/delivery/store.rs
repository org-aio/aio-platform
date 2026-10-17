use anyhow::{Result, ensure};
use az_plugin_delivery::BuildJob;
use sqlx::{PgPool, Row};

pub(in crate::runtime::server) async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::Executor::execute(pool, sqlx::raw_sql(include_str!("schema.sql"))).await?;
    Ok(())
}

pub(super) async fn check_lease(pool: &PgPool, id: i64, lease: &str) -> Result<BuildJob> {
    let row = sqlx::query("SELECT j.git,j.source_revision,j.recipe FROM delivery_jobs j JOIN delivery_sources s ON s.git=j.git WHERE j.id=$1 AND j.lease=$2 AND j.lease_until>now() AND j.state IN ('building','uploaded') AND s.enabled AND s.desired_sha=j.source_revision")
        .bind(id).bind(lease).fetch_optional(pool).await?;
    let row = row.ok_or_else(|| anyhow::anyhow!("构建租约已失效或源码版本已被替代"))?;
    let source_revision: String = row.try_get("source_revision")?;
    ensure!(source_revision.len() == 40, "源码提交无效");
    Ok(BuildJob {
        id,
        lease: lease.into(),
        git: row.try_get("git")?,
        version: format!("0.0.0-dev.{id}+{source_revision}"),
        source_revision,
        recipe: serde_json::from_value(row.try_get("recipe")?)?,
    })
}

// 先锁来源再锁任务，与发现器的更新顺序一致；准备产物后必须在提交事务内再次调用。
pub(super) async fn lock_lease(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    job: &BuildJob,
) -> Result<()> {
    let current: Option<bool> = sqlx::query_scalar(
        "SELECT enabled AND desired_sha=$2 FROM delivery_sources WHERE git=$1 FOR SHARE",
    )
    .bind(&job.git)
    .bind(&job.source_revision)
    .fetch_optional(&mut **tx)
    .await?;
    ensure!(current == Some(true), "构建已被后续源码提交替代");
    let valid: Option<bool> = sqlx::query_scalar("SELECT lease=$2 AND lease_until>clock_timestamp() AND state IN ('building','uploaded') FROM delivery_jobs WHERE id=$1 FOR UPDATE")
        .bind(job.id).bind(&job.lease).fetch_optional(&mut **tx).await?;
    ensure!(valid == Some(true), "构建租约已失效");
    Ok(())
}
