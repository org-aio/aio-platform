use std::path::Path;

use anyhow::{Context, Result, ensure};
use az_plugin_bundle::Bundle;
use az_plugin_contract::CapabilityGrants;
use az_plugin_runtime::{ComponentReleases, ReleaseLease, ReleaseVersion, StoredRelease};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

pub struct SqliteReleases {
    pub pool: SqlitePool,
}

impl SqliteReleases {
    pub async fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            pool: super::sqlite::pool(path).await?,
        })
    }
}

struct Lease {
    transaction: Transaction<'static, Sqlite>,
    version: ReleaseVersion,
}

#[async_trait::async_trait]
impl ReleaseLease for Lease {
    fn version(&self) -> &ReleaseVersion {
        &self.version
    }
    async fn finish(self: Box<Self>) -> Result<()> {
        self.transaction
            .commit()
            .await
            .context("释放夹具发布租约失败")
    }
}

// 使用真实 SQLite 事务验证跨连接的发布排空和持久恢复，不用内存锁伪造后端锁。
#[async_trait::async_trait]
impl ComponentReleases for SqliteReleases {
    async fn initialize(&self) -> Result<()> {
        sqlx::raw_sql("CREATE TABLE IF NOT EXISTS releases(source TEXT,tenant TEXT,revision TEXT,generation INTEGER NOT NULL,archive BLOB,grants TEXT,PRIMARY KEY(source,tenant)); CREATE TABLE IF NOT EXISTS activations(source TEXT,tenant TEXT,generation INTEGER,revision TEXT,PRIMARY KEY(source,tenant,generation))").execute(&self.pool).await?;
        Ok(())
    }
    async fn generation(&self, source: Uuid, tenant: &str) -> Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT generation FROM releases WHERE source=? AND tenant=?")
                .bind(source.to_string())
                .bind(tenant)
                .fetch_optional(&self.pool)
                .await?
                .unwrap_or(0),
        )
    }
    async fn stored(&self, source: Uuid, tenant: &str) -> Result<Option<StoredRelease>> {
        let row = sqlx::query("SELECT revision,generation,archive,grants FROM releases WHERE source=? AND tenant=? AND revision IS NOT NULL").bind(source.to_string()).bind(tenant).fetch_optional(&self.pool).await?;
        row.map(|row| {
            let bundle = Bundle::decode(&row.try_get::<Vec<u8>, _>("archive")?)?;
            ensure!(
                bundle.digest == row.try_get::<String, _>("revision")?,
                "夹具整包摘要不符"
            );
            Ok(StoredRelease {
                generation: row.try_get("generation")?,
                bundle,
                grants: serde_json::from_str(&row.try_get::<String, _>("grants")?)?,
            })
        })
        .transpose()
    }
    async fn lease(&self, source: Uuid, tenant: &str) -> Result<Box<dyn ReleaseLease>> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let row =
            sqlx::query("SELECT revision,generation FROM releases WHERE source=? AND tenant=?")
                .bind(source.to_string())
                .bind(tenant)
                .fetch_optional(&mut *transaction)
                .await?;
        let version = match row {
            Some(row) => ReleaseVersion {
                revision: row.try_get("revision")?,
                generation: row.try_get("generation")?,
            },
            None => ReleaseVersion {
                revision: None,
                generation: 0,
            },
        };
        Ok(Box::new(Lease {
            transaction,
            version,
        }))
    }
    async fn activate(
        &self,
        source: Uuid,
        tenant: &str,
        expected: i64,
        bundle: &Bundle,
        grants: &CapabilityGrants,
    ) -> Result<()> {
        let archive = bundle.encode()?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let generation =
            sqlx::query_scalar("SELECT generation FROM releases WHERE source=? AND tenant=?")
                .bind(source.to_string())
                .bind(tenant)
                .fetch_optional(&mut *transaction)
                .await?
                .unwrap_or(0i64);
        ensure!(generation == expected, "并发激活已经改变代数");
        let next = generation.checked_add(1).context("夹具代数耗尽")?;
        sqlx::query("INSERT INTO releases(source,tenant,revision,generation,archive,grants) VALUES(?,?,?,?,?,?) ON CONFLICT(source,tenant) DO UPDATE SET revision=excluded.revision,generation=excluded.generation,archive=excluded.archive,grants=excluded.grants").bind(source.to_string()).bind(tenant).bind(&bundle.digest).bind(next).bind(archive).bind(serde_json::to_string(grants)?).execute(&mut *transaction).await?;
        audit(&mut transaction, source, tenant, next, Some(&bundle.digest)).await?;
        transaction.commit().await?;
        Ok(())
    }
    async fn deactivate(&self, source: Uuid, tenant: &str) -> Result<()> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let row = sqlx::query(
            "SELECT generation FROM releases WHERE source=? AND tenant=? AND revision IS NOT NULL",
        )
        .bind(source.to_string())
        .bind(tenant)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(row) = row {
            let generation: i64 = row.try_get("generation")?;
            let next = generation.checked_add(1).context("夹具代数耗尽")?;
            sqlx::query("UPDATE releases SET revision=NULL,generation=?,archive=NULL,grants=NULL WHERE source=? AND tenant=?").bind(next).bind(source.to_string()).bind(tenant).execute(&mut *transaction).await?;
            audit(&mut transaction, source, tenant, next, None).await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}

async fn audit(
    transaction: &mut Transaction<'static, Sqlite>,
    source: Uuid,
    tenant: &str,
    generation: i64,
    revision: Option<&str>,
) -> Result<()> {
    sqlx::query("INSERT INTO activations VALUES(?,?,?,?)")
        .bind(source.to_string())
        .bind(tenant)
        .bind(generation)
        .bind(revision)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}
