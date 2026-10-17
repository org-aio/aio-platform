use anyhow::{Context, Result, ensure};
use az_plugin_bundle::Bundle;
use az_plugin_contract::CapabilityGrants;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::{ComponentReleases, ReleaseLease, ReleaseVersion, StoredRelease};

/// 默认发布存储；沿用既有表、代数、审计与数据库锁，租户数据角色无法读取。
pub struct PostgresReleases {
    pool: PgPool,
}

impl PostgresReleases {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

struct PostgresLease {
    transaction: Transaction<'static, Postgres>,
    version: ReleaseVersion,
}

#[async_trait::async_trait]
impl ReleaseLease for PostgresLease {
    fn version(&self) -> &ReleaseVersion {
        &self.version
    }
    async fn finish(self: Box<Self>) -> Result<()> {
        self.transaction
            .commit()
            .await
            .context("释放发布读租约失败")
    }
}

#[async_trait::async_trait]
impl ComponentReleases for PostgresReleases {
    async fn initialize(&self) -> Result<()> {
        let pool = &self.pool;
        let mut tx = pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('aio-plugin-bindings',0))")
            .execute(&mut *tx)
            .await?;
        for statement in [
            "CREATE SCHEMA IF NOT EXISTS aio_plugin_host",
            "REVOKE ALL ON SCHEMA aio_plugin_host FROM PUBLIC",
            "CREATE TABLE IF NOT EXISTS aio_plugin_host.component_packages(digest TEXT PRIMARY KEY,archive BYTEA NOT NULL)",
            "CREATE TABLE IF NOT EXISTS aio_plugin_host.component_releases(source_id UUID NOT NULL,tenant_id TEXT NOT NULL,revision TEXT REFERENCES aio_plugin_host.component_packages(digest),generation BIGINT NOT NULL,grants JSONB NOT NULL,PRIMARY KEY(source_id,tenant_id))",
            "CREATE TABLE IF NOT EXISTS aio_plugin_host.component_activations(source_id UUID NOT NULL,tenant_id TEXT NOT NULL,generation BIGINT NOT NULL,revision TEXT,created_at TIMESTAMPTZ NOT NULL DEFAULT now(),PRIMARY KEY(source_id,tenant_id,generation))",
        ] {
            sqlx::query(statement).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    async fn generation(&self, source: Uuid, tenant: &str) -> Result<i64> {
        let pool = &self.pool;
        Ok(sqlx::query_scalar("SELECT generation FROM aio_plugin_host.component_releases WHERE source_id=$1 AND tenant_id=$2")
        .bind(source).bind(tenant).fetch_optional(pool).await?.unwrap_or(0))
    }
    async fn stored(&self, source: Uuid, tenant: &str) -> Result<Option<StoredRelease>> {
        let pool = &self.pool;
        let row = sqlx::query("SELECT r.generation,r.grants,r.revision,p.archive FROM aio_plugin_host.component_releases r JOIN aio_plugin_host.component_packages p ON p.digest=r.revision WHERE r.source_id=$1 AND r.tenant_id=$2")
        .bind(source).bind(tenant).fetch_optional(pool).await?;
        row.map(|row| {
            let bundle = Bundle::decode(&row.get::<Vec<u8>, _>("archive"))?;
            ensure!(
                bundle.digest == row.get::<String, _>("revision"),
                "持久整包摘要不匹配"
            );
            Ok(StoredRelease {
                generation: row.get("generation"),
                bundle,
                grants: serde_json::from_value(row.get("grants"))?,
            })
        })
        .transpose()
    }
    async fn activate(
        &self,
        source: Uuid,
        tenant: &str,
        expected: i64,
        bundle: &Bundle,
        grants: &CapabilityGrants,
    ) -> Result<()> {
        let pool = &self.pool;
        let archive = bundle.encode()?;
        let revision = &bundle.digest;
        let mut tx = pool.begin().await?;
        lock(&mut tx, source, tenant).await?;
        let (_, generation) = locked_revision(&mut tx, source, tenant, true).await?;
        ensure!(generation == expected, "并发发布已改变活动版本，请重新校验");
        let next = generation.checked_add(1).context("版本代数已耗尽")?;
        sqlx::query("INSERT INTO aio_plugin_host.component_packages(digest,archive) VALUES($1,$2) ON CONFLICT DO NOTHING").bind(revision).bind(archive).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO aio_plugin_host.component_releases(source_id,tenant_id,revision,generation,grants) VALUES($1,$2,$3,$4,$5) ON CONFLICT(source_id,tenant_id) DO UPDATE SET revision=EXCLUDED.revision,generation=EXCLUDED.generation,grants=EXCLUDED.grants")
        .bind(source).bind(tenant).bind(revision).bind(next).bind(serde_json::to_value(grants)?).execute(&mut *tx).await?;
        audit(&mut tx, source, tenant, next, Some(revision)).await?;
        tx.commit().await.context("提交持久活动版本失败")
    }
    async fn deactivate(&self, source: Uuid, tenant: &str) -> Result<()> {
        let pool = &self.pool;
        let mut tx = pool.begin().await?;
        lock(&mut tx, source, tenant).await?;
        let (revision, generation) = locked_revision(&mut tx, source, tenant, true).await?;
        if revision.is_some() {
            let next = generation.checked_add(1).context("版本代数已耗尽")?;
            sqlx::query("UPDATE aio_plugin_host.component_releases SET revision=NULL,generation=$3,grants='{}' WHERE source_id=$1 AND tenant_id=$2").bind(source).bind(tenant).bind(next).execute(&mut *tx).await?;
            audit(&mut tx, source, tenant, next, None).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn lease(&self, source: Uuid, tenant: &str) -> Result<Box<dyn ReleaseLease>> {
        let mut transaction = self.pool.begin().await?;
        let (revision, generation) =
            locked_revision(&mut transaction, source, tenant, false).await?;
        Ok(Box::new(PostgresLease {
            transaction,
            version: ReleaseVersion {
                revision,
                generation,
            },
        }))
    }
}

async fn locked_revision(
    tx: &mut Transaction<'_, Postgres>,
    source: Uuid,
    tenant: &str,
    exclusive: bool,
) -> Result<(Option<String>, i64)> {
    let sql = if exclusive {
        "SELECT revision,generation FROM aio_plugin_host.component_releases WHERE source_id=$1 AND tenant_id=$2 FOR UPDATE"
    } else {
        "SELECT revision,generation FROM aio_plugin_host.component_releases WHERE source_id=$1 AND tenant_id=$2 FOR SHARE"
    };
    Ok(sqlx::query(sql)
        .bind(source)
        .bind(tenant)
        .fetch_optional(&mut **tx)
        .await?
        .map(|row| (row.get("revision"), row.get("generation")))
        .unwrap_or((None, 0)))
}

async fn lock(tx: &mut Transaction<'_, Postgres>, source: Uuid, tenant: &str) -> Result<()> {
    sqlx::query("SET LOCAL lock_timeout = '20s'")
        .execute(&mut **tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(serde_json::to_string(&(source, tenant))?)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    source: Uuid,
    tenant: &str,
    generation: i64,
    revision: Option<&str>,
) -> Result<()> {
    sqlx::query("INSERT INTO aio_plugin_host.component_activations(source_id,tenant_id,generation,revision) VALUES($1,$2,$3,$4)")
        .bind(source).bind(tenant).bind(generation).bind(revision).execute(&mut **tx).await?;
    Ok(())
}
