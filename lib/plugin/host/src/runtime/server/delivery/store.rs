use anyhow::{Result, ensure};
use az_plugin_delivery::{BuildJob, BuildRecipe};
use sqlx::{PgPool, Row};

pub(in crate::runtime::server) async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql(include_str!("schema.sql"))
        .execute(pool)
        .await?;
    Ok(())
}

pub(super) async fn claim(pool: &PgPool) -> Result<Option<BuildJob>> {
    let mut tx = pool.begin().await?;
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
