use anyhow::Result;
use sqlx::PgPool;

use super::super::enhancement_catalog::Observation;

pub(super) async fn started(
    pool: &PgPool,
    tenant: &str,
    revision: &str,
    generation: i64,
    path: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO runtime_enhancement_observations(tenant_id,revision,generation,path,started) VALUES($1,$2,$3,$4,1) ON CONFLICT(tenant_id,generation,path) DO UPDATE SET started=runtime_enhancement_observations.started+1,updated_at=now()")
        .bind(tenant).bind(revision).bind(generation).bind(path).execute(pool).await?;
    Ok(())
}

pub(super) async fn finished(
    pool: &PgPool,
    tenant: &str,
    generation: i64,
    path: &str,
    success: bool,
) -> Result<()> {
    sqlx::query("UPDATE runtime_enhancement_observations SET succeeded=succeeded+$4,failed=failed+$5,updated_at=now() WHERE tenant_id=$1 AND generation=$2 AND path=$3")
        .bind(tenant).bind(generation).bind(path).bind(i64::from(success)).bind(i64::from(!success))
        .execute(pool).await?;
    Ok(())
}

pub(super) async fn list(pool: &PgPool, tenant: &str, generation: i64) -> Result<Vec<Observation>> {
    Ok(sqlx::query_as("SELECT path,started,succeeded,failed,(extract(epoch FROM updated_at)*1000)::bigint AS updated_at FROM runtime_enhancement_observations WHERE tenant_id=$1 AND generation=$2 ORDER BY path")
        .bind(tenant).bind(generation).fetch_all(pool).await?)
}
