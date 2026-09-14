use anyhow::{Context as _, Result, ensure};
use sha2::{Digest as _, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction};

use super::store::PluginStore;
use crate::runtime::{PluginRuntime, PublishCredentialView, PublishState};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS plugin_publish_credentials (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    git TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at TIMESTAMPTZ,
    UNIQUE(tenant_id, git)
);
CREATE TABLE IF NOT EXISTS plugin_publish_jobs (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    git TEXT NOT NULL,
    revision TEXT NOT NULL,
    runtime TEXT NOT NULL,
    page_count INTEGER NOT NULL,
    state TEXT NOT NULL,
    detail TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(tenant_id, git, revision)
);
CREATE INDEX IF NOT EXISTS plugin_publish_jobs_pending_idx ON plugin_publish_jobs(state, updated_at);
CREATE SEQUENCE IF NOT EXISTS plugin_publish_requests_seq;
ALTER TABLE plugin_publish_jobs ADD COLUMN IF NOT EXISTS request_order BIGINT NOT NULL DEFAULT nextval('plugin_publish_requests_seq');
"#;

#[derive(Clone)]
pub(super) struct PublishJob {
    pub id: String,
    pub tenant_id: String,
    pub source_id: String,
    pub git: String,
    pub revision: String,
    pub runtime: PluginRuntime,
    pub page_count: usize,
    pub state: PublishState,
    pub detail: String,
}

pub(super) struct PublisherBinding {
    pub tenant_id: String,
    pub git: String,
}

pub(super) async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql(SCHEMA)
        .execute(pool)
        .await
        .context("创建插件发布凭据表失败")?;
    Ok(())
}

async fn lock_publication_source(
    transaction: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    git: &str,
) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 1729))")
        .bind(serde_json::to_string(&(tenant_id, git))?)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

pub(super) async fn ensure_current_publication(
    transaction: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    git: &str,
    revision: &str,
) -> Result<()> {
    lock_publication_source(transaction, tenant_id, git).await?;
    let current: Option<String> = sqlx::query_scalar(
        "SELECT revision FROM plugin_publish_jobs WHERE tenant_id = $1 AND git = $2 ORDER BY request_order DESC LIMIT 1",
    ).bind(tenant_id).bind(git).fetch_optional(&mut **transaction).await?;
    ensure!(
        current.as_deref() == Some(revision),
        "当前发布已被后续请求替代，保留现有活动版本"
    );
    Ok(())
}

impl PluginStore {
    pub(super) async fn create_publish_credential(
        &self,
        tenant_id: &str,
        git: &str,
    ) -> Result<PublishCredentialView> {
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let token_hash = token_hash(&token);
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO plugin_publish_credentials (id, tenant_id, git, token_hash) VALUES ($1, $2, $3, $4) ON CONFLICT (tenant_id, git) DO UPDATE SET id = EXCLUDED.id, token_hash = EXCLUDED.token_hash, created_at = now(), revoked_at = NULL",
        )
        .bind(&id)
        .bind(tenant_id)
        .bind(git)
        .bind(token_hash)
        .execute(&self.pool)
        .await?;
        Ok(PublishCredentialView {
            id,
            tenant_id: tenant_id.to_owned(),
            git: git.to_owned(),
            token,
        })
    }

    pub(super) async fn publisher_binding(&self, token: &str) -> Result<Option<PublisherBinding>> {
        let row = sqlx::query(
            "SELECT tenant_id, git FROM plugin_publish_credentials WHERE token_hash = $1 AND revoked_at IS NULL",
        )
        .bind(token_hash(token))
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| PublisherBinding {
            tenant_id: row.get("tenant_id"),
            git: row.get("git"),
        }))
    }

    pub(super) async fn revoke_publish_credential(
        &self,
        tenant_id: &str,
        credential_id: &str,
    ) -> Result<()> {
        let result = sqlx::query(
            "UPDATE plugin_publish_credentials SET revoked_at = now() WHERE id = $1 AND tenant_id = $2 AND revoked_at IS NULL",
        )
        .bind(credential_id)
        .bind(tenant_id)
        .execute(&self.pool)
        .await?;
        ensure!(result.rows_affected() == 1, "发布凭证不存在或已撤销");
        Ok(())
    }

    pub(super) async fn queue_publish_job(
        &self,
        tenant_id: &str,
        source_id: &str,
        git: &str,
        revision: &str,
        runtime: PluginRuntime,
        page_count: usize,
    ) -> Result<PublishJob> {
        let id = uuid::Uuid::new_v4().to_string();
        let mut transaction = self.pool.begin().await?;
        lock_publication_source(&mut transaction, tenant_id, git).await?;
        let row = sqlx::query(
            "INSERT INTO plugin_publish_jobs (id, tenant_id, source_id, git, revision, runtime, page_count, state, detail) VALUES ($1, $2, $3, $4, $5, $6, $7, 'queued', '已持久化 artifact，等待后台验证') ON CONFLICT (tenant_id, git, revision) DO UPDATE SET source_id = CASE WHEN plugin_publish_jobs.state = 'running' THEN plugin_publish_jobs.source_id ELSE EXCLUDED.source_id END, runtime = CASE WHEN plugin_publish_jobs.state = 'running' THEN plugin_publish_jobs.runtime ELSE EXCLUDED.runtime END, page_count = CASE WHEN plugin_publish_jobs.state = 'running' THEN plugin_publish_jobs.page_count ELSE EXCLUDED.page_count END, state = CASE WHEN plugin_publish_jobs.state = 'running' THEN 'running' ELSE 'queued' END, detail = CASE WHEN plugin_publish_jobs.state = 'running' THEN plugin_publish_jobs.detail ELSE '已持久化 artifact，等待后台验证' END, request_order = nextval('plugin_publish_requests_seq'), updated_at = now() RETURNING id, tenant_id, source_id, git, revision, runtime, page_count, state, detail",
        )
        .bind(id)
        .bind(tenant_id)
        .bind(source_id)
        .bind(git)
        .bind(revision)
        .bind(super::store::runtime_name(runtime))
        .bind(i32::try_from(page_count).context("发布页面数量超过数据库范围")?)
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;
        publish_job(row)
    }

    pub(super) async fn claim_publish_job(&self, job_id: &str) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE plugin_publish_jobs SET state = 'running', detail = '正在验证 Component 并执行健康检查', updated_at = now() WHERE id = $1 AND state = 'queued'",
        )
        .bind(job_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub(super) async fn finish_publish_job(
        &self,
        job_id: &str,
        state: PublishState,
        detail: &str,
        page_count: Option<usize>,
    ) -> Result<()> {
        ensure!(
            matches!(state, PublishState::Active | PublishState::Failed),
            "发布任务只能完成为 active 或 failed"
        );
        let page_count = page_count
            .map(|value| i32::try_from(value).context("发布页面数量超过数据库范围"))
            .transpose()?;
        let result = sqlx::query(
            "UPDATE plugin_publish_jobs SET state = $2, detail = $3, page_count = COALESCE($4, page_count), updated_at = now() WHERE id = $1 AND state = 'running'",
        )
        .bind(job_id)
        .bind(publish_state_name(state))
        .bind(detail)
        .bind(page_count)
        .execute(&self.pool)
        .await?;
        ensure!(result.rows_affected() == 1, "发布任务已被其他执行批次接管");
        Ok(())
    }

    pub(super) async fn resume_publish_jobs(&self) -> Result<Vec<PublishJob>> {
        sqlx::query(
            "UPDATE plugin_publish_jobs SET state = 'queued', detail = '宿主重启后恢复后台验证', updated_at = now() WHERE state = 'running'",
        )
        .execute(&self.pool)
        .await?;
        let rows = sqlx::query(
            "SELECT id, tenant_id, source_id, git, revision, runtime, page_count, state, detail FROM plugin_publish_jobs WHERE state = 'queued' ORDER BY created_at",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(publish_job).collect()
    }

    pub(super) async fn publish_job(
        &self,
        tenant_id: &str,
        job_id: &str,
    ) -> Result<Option<PublishJob>> {
        sqlx::query(
            "SELECT id, tenant_id, source_id, git, revision, runtime, page_count, state, detail FROM plugin_publish_jobs WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant_id)
        .bind(job_id)
        .fetch_optional(&self.pool)
        .await?
        .map(publish_job)
        .transpose()
    }
}

fn publish_job(row: sqlx::postgres::PgRow) -> Result<PublishJob> {
    let page_count = row.try_get::<i32, _>("page_count")?;
    ensure!(page_count >= 0, "发布任务页面数量不能为负数");
    Ok(PublishJob {
        id: row.try_get("id")?,
        tenant_id: row.try_get("tenant_id")?,
        source_id: row.try_get("source_id")?,
        git: row.try_get("git")?,
        revision: row.try_get("revision")?,
        runtime: super::store::parse_runtime(row.try_get("runtime")?)?,
        page_count: usize::try_from(page_count).context("发布任务页面数量无效")?,
        state: parse_publish_state(row.try_get("state")?)?,
        detail: row.try_get("detail")?,
    })
}

fn publish_state_name(state: PublishState) -> &'static str {
    match state {
        PublishState::Queued => "queued",
        PublishState::Running => "running",
        PublishState::Active => "active",
        PublishState::Failed => "failed",
    }
}

fn parse_publish_state(value: String) -> Result<PublishState> {
    match value.as_str() {
        "queued" => Ok(PublishState::Queued),
        "running" => Ok(PublishState::Running),
        "active" => Ok(PublishState::Active),
        "failed" => Ok(PublishState::Failed),
        _ => anyhow::bail!("未知发布任务状态: {value}"),
    }
}

fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}
