use std::time::Duration;

use anyhow::{Result, ensure};
use sqlx::{Postgres, Row, Transaction};

use super::super::{RuntimeState, installation};
use crate::runtime::InstallPluginRequest;

pub(in crate::runtime::server) async fn ensure_current(
    tx: &mut Transaction<'_, Postgres>,
    git: &str,
    revision: &str,
) -> Result<()> {
    let current: Option<bool> = sqlx::query_scalar("SELECT s.enabled AND s.desired_sha=j.source_revision FROM delivery_jobs j JOIN delivery_sources s ON s.git=j.git WHERE j.package_revision=$1 AND j.git=$2 FOR SHARE OF s")
        .bind(revision).bind(git).fetch_optional(&mut **tx).await?;
    ensure!(current != Some(false), "构建已被后续源码提交替代");
    Ok(())
}

pub(in crate::runtime::server) async fn remember_installation(
    tx: &mut Transaction<'_, Postgres>,
    tenant: &str,
    source: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO delivery_installations(tenant_id,source_id) VALUES($1,$2) ON CONFLICT DO NOTHING").bind(tenant).bind(source).execute(&mut **tx).await?;
    Ok(())
}

pub(in crate::runtime::server) async fn exclude_revision(
    tx: &mut Transaction<'_, Postgres>,
    tenant: &str,
    source: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO delivery_installations(tenant_id,source_id,excluded_revision) SELECT $1,$2,m.rev FROM marketplace_entries m JOIN plugin_sources s ON s.git=m.git WHERE s.id=$2 AND m.source='aio://published' ON CONFLICT(tenant_id,source_id) DO UPDATE SET excluded_revision=EXCLUDED.excluded_revision")
        .bind(tenant).bind(source).execute(&mut **tx).await?;
    Ok(())
}

pub(super) async fn tick(state: &RuntimeState) -> Result<()> {
    sqlx::query("UPDATE delivery_jobs j SET state=CASE p.state WHEN 'active' THEN 'active' ELSE 'failed' END,error=CASE WHEN p.state='failed' THEN p.detail ELSE NULL END,updated_at=now() FROM plugin_publish_jobs p WHERE j.package_revision=p.revision AND j.state='publishing' AND p.state IN ('active','failed')").execute(&state.store.pool).await?;
    sqlx::query("INSERT INTO delivery_rollouts(tenant_id,source_id,revision) SELECT b.tenant_id,b.source_id,m.rev FROM tenant_plugin_bindings b JOIN plugin_sources s ON s.id=b.source_id JOIN marketplace_entries m ON m.git=s.git AND m.source='aio://published' JOIN plugin_revisions r ON r.id=b.revision_id LEFT JOIN delivery_installations i ON i.tenant_id=b.tenant_id AND i.source_id=b.source_id WHERE b.enabled AND r.revision<>m.rev AND i.excluded_revision IS DISTINCT FROM m.rev ON CONFLICT DO NOTHING").execute(&state.store.pool).await?;
    let rows = sqlx::query("SELECT q.tenant_id,q.source_id,q.revision,s.git FROM delivery_rollouts q JOIN tenant_plugin_bindings b ON b.tenant_id=q.tenant_id AND b.source_id=q.source_id JOIN plugin_sources s ON s.id=q.source_id JOIN marketplace_entries m ON m.git=s.git AND m.source='aio://published' AND m.rev=q.revision LEFT JOIN delivery_installations i ON i.tenant_id=q.tenant_id AND i.source_id=q.source_id WHERE b.enabled AND i.excluded_revision IS DISTINCT FROM q.revision AND (q.state='queued' OR (q.state='failed' AND q.updated_at<now()-interval '1 minute')) LIMIT 8")
        .fetch_all(&state.store.pool).await?;
    for row in rows {
        let tenant: String = row.try_get("tenant_id")?;
        let source: String = row.try_get("source_id")?;
        let revision: String = row.try_get("revision")?;
        let result = installation::install_followed(
            state,
            &tenant,
            &InstallPluginRequest {
                git: row.try_get("git")?,
                rev: Some(revision.clone()),
            },
        )
        .await;
        sqlx::query("UPDATE delivery_rollouts SET state=$4,error=$5,updated_at=now() WHERE tenant_id=$1 AND source_id=$2 AND revision=$3")
            .bind(&tenant).bind(&source).bind(&revision).bind(if result.is_ok() { "active" } else { "failed" }).bind(result.err().map(|e| format!("{e:#}"))).execute(&state.store.pool).await?;
    }
    // 保留活动和最近版本；删除仅作用于不再引用的二进制归档。
    let expired: Vec<String> = sqlx::query_scalar("DELETE FROM plugin_packages p WHERE p.revision IN (SELECT revision FROM (SELECT p.revision,row_number() OVER(PARTITION BY p.git ORDER BY p.created_at DESC) AS position FROM plugin_packages p WHERE EXISTS(SELECT 1 FROM plugin_revisions r WHERE r.revision=p.revision)) ranked WHERE position>10) AND NOT EXISTS(SELECT 1 FROM tenant_plugin_bindings b JOIN plugin_revisions r ON r.id=b.revision_id WHERE r.revision=p.revision) AND NOT EXISTS(SELECT 1 FROM marketplace_entries m WHERE m.rev=p.revision) AND NOT EXISTS(SELECT 1 FROM delivery_jobs j WHERE j.package_revision=p.revision AND j.state IN ('building','uploaded','publishing')) AND NOT EXISTS(SELECT 1 FROM plugin_publish_jobs j WHERE j.revision=p.revision AND j.state IN ('queued','running')) AND NOT EXISTS(SELECT 1 FROM delivery_installations i WHERE i.excluded_revision=p.revision) RETURNING p.revision").fetch_all(&state.store.pool).await?;
    if !expired.is_empty() {
        let _guard = state.repository.cache_operations.lock().await;
        for revision in expired {
            if revision.len() == 64 && revision.bytes().all(|b| b.is_ascii_hexdigit()) {
                let path = state.repository.cache_root.join(revision);
                if path.exists() {
                    tokio::fs::remove_dir_all(path).await?;
                }
            }
        }
    }
    Ok(())
}

pub(super) async fn run(state: RuntimeState) {
    loop {
        if let Err(error) = tick(&state).await {
            eprintln!("插件自动升级失败: {error:#}");
        }
        tokio::time::sleep(Duration::from_secs(15)).await;
    }
}
