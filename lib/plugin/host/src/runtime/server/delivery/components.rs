use super::super::RuntimeState;
use anyhow::{Result, ensure};
use az_plugin_bundle::Bundle;
use az_plugin_delivery::{BuildJob, Documentation};
use sqlx::{Postgres, Transaction};

pub(super) async fn upload(state: &RuntimeState, job: &BuildJob, bytes: &[u8]) -> Result<()> {
    let body = bytes.to_vec();
    let bundle = tokio::task::spawn_blocking(move || Bundle::decode(&body)).await??;
    ensure!(
        bundle.git == job.git
            && bundle.commit == job.source_revision
            && bundle.version == job.version,
        "构建产物与任务的仓库、源码提交或版本不符"
    );
    let mut tx = state.store.pool.begin().await?;
    // 与发现器共用来源行锁，避免校验租约后较旧上传覆盖新目标。
    let current: bool = sqlx::query_scalar(
        "SELECT enabled AND desired_sha=$2 FROM delivery_sources WHERE git=$1 FOR SHARE",
    )
    .bind(&job.git)
    .bind(&job.source_revision)
    .fetch_one(&mut *tx)
    .await?;
    ensure!(current, "构建已被后续源码提交替代");
    let updated = sqlx::query("UPDATE delivery_jobs SET package_revision=$3,state='uploaded',lease_until=now()+interval '5 minutes',updated_at=now() WHERE id=$1 AND lease=$2 AND lease_until>now() AND state IN ('building','uploaded')")
        .bind(job.id).bind(&job.lease).bind(&bundle.digest).execute(&mut *tx).await?.rows_affected();
    ensure!(updated == 1, "构建租约已失效");
    sqlx::query("INSERT INTO delivery_component_packages(job_id,archive) VALUES($1,$2) ON CONFLICT(job_id) DO UPDATE SET archive=EXCLUDED.archive")
        .bind(job.id).bind(bytes).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn complete(
    state: &RuntimeState,
    job: &BuildJob,
    documentation: &Documentation,
) -> Result<bool> {
    ensure!(documentation.readme.len() <= 512 * 1024, "README 超过限制");
    let mut tx = state.store.pool.begin().await?;
    let updated = sqlx::query("UPDATE delivery_component_packages SET readme=$2 WHERE job_id=$1")
        .bind(job.id)
        .bind(&documentation.readme)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if updated == 0 {
        return Ok(false);
    }
    let updated = sqlx::query("UPDATE delivery_jobs SET state='publishing',lease_until=NULL,updated_at=now() WHERE id=$1 AND lease=$2 AND state='uploaded' AND lease_until>now()")
        .bind(job.id).bind(&job.lease).execute(&mut *tx).await?.rows_affected();
    ensure!(updated == 1, "构建租约已失效");
    tx.commit().await?;
    Ok(true)
}

pub(in crate::runtime::server) async fn finish_publication(
    tx: &mut Transaction<'_, Postgres>,
    id: i64,
    bundle: &Bundle,
) -> Result<()> {
    let current: bool = sqlx::query_scalar(
        "SELECT enabled AND desired_sha=$2 FROM delivery_sources WHERE git=$1 FOR SHARE",
    )
    .bind(&bundle.git)
    .bind(&bundle.commit)
    .fetch_one(&mut **tx)
    .await?;
    ensure!(current, "构建已被后续源码提交替代");
    let updated = sqlx::query("UPDATE delivery_jobs SET state='active',error=NULL,updated_at=now() WHERE id=$1 AND state='publishing' AND git=$2 AND source_revision=$3 AND package_revision=$4")
        .bind(id).bind(&bundle.git).bind(&bundle.commit).bind(&bundle.digest).execute(&mut **tx).await?.rows_affected();
    ensure!(updated == 1, "交付发布任务已失效");
    sqlx::query("DELETE FROM delivery_component_packages WHERE job_id=$1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(in crate::runtime::server) async fn tick(state: &RuntimeState) -> Result<()> {
    let Some(components) = &state.components else {
        return Ok(());
    };
    let pending = sqlx::query_as::<_, (i64, Vec<u8>, String)>("SELECT j.id,p.archive,p.readme FROM delivery_jobs j JOIN delivery_component_packages p ON p.job_id=j.id WHERE j.state='publishing' ORDER BY j.id LIMIT 1")
        .fetch_optional(&state.store.pool).await?;
    if let Some((id, bytes, readme)) = pending {
        let result = async {
            let bundle = tokio::task::spawn_blocking(move || Bundle::decode(&bytes)).await??;
            components.publish_delivery(bundle, &readme, id).await?;
            anyhow::Ok(())
        }
        .await;
        if let Err(error) = result {
            sqlx::query("UPDATE delivery_jobs SET state='failed',error=$2,updated_at=now() WHERE id=$1 AND state='publishing'")
                .bind(id).bind(format!("{error:#}").chars().take(16000).collect::<String>()).execute(&state.store.pool).await?;
        }
    }
    sqlx::query("DELETE FROM delivery_component_packages p USING delivery_jobs j WHERE p.job_id=j.id AND j.state IN ('superseded','active')").execute(&state.store.pool).await?;
    components.rollout().await
}
