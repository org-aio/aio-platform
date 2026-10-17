use anyhow::{Context, Result, ensure};
use az_plugin_enhancement::Package;
use sqlx::PgPool;

use super::{
    Activation, CommitAuthorization, EnhancementCatalog, Event, Head, Observation, Revision,
    observations,
};

/// 默认增强存储；使用现有表，并在同一事务内保存指针、历史和提交前授权结果。
pub struct PostgresEnhancementCatalog {
    pool: PgPool,
}

impl PostgresEnhancementCatalog {
    /// 初始化增强表；连接池由 DatabasePlugin 管理并统一关闭。
    pub async fn open(pool: PgPool) -> Result<Self> {
        let mut transaction = pool.begin().await?;
        sqlx::Executor::execute(&mut *transaction, sqlx::raw_sql(include_str!("schema.sql")))
            .await?;
        transaction.commit().await?;
        Ok(Self { pool })
    }
}

#[async_trait::async_trait]
impl EnhancementCatalog for PostgresEnhancementCatalog {
    async fn head(&self, tenant: &str) -> Result<Head> {
        Ok(sqlx::query_as(
            "SELECT revision,generation FROM runtime_enhancement_heads WHERE tenant_id=$1",
        )
        .bind(tenant)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or_default())
    }

    async fn revisions(&self, tenant: &str) -> Result<Vec<Revision>> {
        Ok(sqlx::query_as("SELECT revision,author,(extract(epoch FROM created_at)*1000)::bigint AS created_at FROM runtime_enhancement_revisions WHERE tenant_id=$1 ORDER BY created_at DESC,revision LIMIT 50")
            .bind(tenant).fetch_all(&self.pool).await?)
    }

    async fn package(&self, tenant: &str, revision: &str) -> Result<Package> {
        let value: serde_json::Value = sqlx::query_scalar(
            "SELECT package FROM runtime_enhancement_revisions WHERE tenant_id=$1 AND revision=$2",
        )
        .bind(tenant)
        .bind(revision)
        .fetch_optional(&self.pool)
        .await?
        .context("当前租户没有该增强版本")?;
        serde_json::from_value(value).context("读取增强包失败")
    }

    async fn history(&self, tenant: &str) -> Result<Vec<Event>> {
        Ok(sqlx::query_as("SELECT generation,revision,actor,action,(extract(epoch FROM created_at)*1000)::bigint AS created_at FROM runtime_enhancement_events WHERE tenant_id=$1 ORDER BY generation DESC LIMIT 50")
            .bind(tenant).fetch_all(&self.pool).await?)
    }

    async fn observe(
        &self,
        tenant: &str,
        revision: &str,
        generation: i64,
    ) -> Result<Vec<Observation>> {
        // 先校验版本归属，不能用观察查询探测其他租户的摘要。
        let known: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runtime_enhancement_events WHERE tenant_id=$1 AND revision=$2 AND generation=$3)")
            .bind(tenant).bind(revision).bind(generation).fetch_one(&self.pool).await?;
        ensure!(known, "当前租户没有该版本与代数的激活记录");
        observations::list(&self.pool, tenant, generation).await
    }

    async fn propose(
        &self,
        tenant: &str,
        author: &str,
        package: &Package,
        authorize: CommitAuthorization<'_>,
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("INSERT INTO runtime_enhancement_revisions(tenant_id,revision,package,author) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING")
            .bind(tenant).bind(package.revision()?).bind(serde_json::to_value(package)?).bind(author)
            .execute(&mut *transaction).await?;
        // 编译和数据库行锁等待都可能耗时；提交前重新核验，失败不保存候选。
        authorize.await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn activate(
        &self,
        tenant: &str,
        actor: &str,
        request: &Activation,
        rollback: bool,
        authorize: CommitAuthorization<'_>,
    ) -> Result<Option<Head>> {
        let revision = request.revision.as_deref();
        let expected_generation = request.expected_generation;
        ensure!(expected_generation >= 0, "活动代数无效");
        let contracts = match revision {
            Some(revision) => self
                .package(tenant, revision)
                .await?
                .enhancements
                .iter()
                .map(|entry| entry.contract())
                .collect::<Vec<_>>(),
            None => Vec::new(),
        };
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO runtime_enhancement_heads(tenant_id) VALUES($1) ON CONFLICT DO NOTHING",
        )
        .bind(tenant)
        .execute(&mut *transaction)
        .await?;
        let next: Option<Head> = sqlx::query_as("UPDATE runtime_enhancement_heads SET revision=$2,generation=generation+1 WHERE tenant_id=$1 AND generation=$3 RETURNING revision,generation")
            .bind(tenant).bind(revision).bind(expected_generation).fetch_optional(&mut *transaction).await?;
        let Some(next) = next else {
            return Ok(None);
        };
        if rollback {
            let revision = revision.context("回滚必须指定曾激活的版本")?;
            let known: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runtime_enhancement_events WHERE tenant_id=$1 AND revision=$2)")
                .bind(tenant).bind(revision).fetch_one(&mut *transaction).await?;
            ensure!(known, "只能回滚到当前租户曾激活的版本");
        }
        // CAS 已串行化当前租户发布；直接查询不可变历史，停用或回滚也不能释放已用契约。
        let conflict: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM runtime_enhancement_revisions r
                CROSS JOIN LATERAL jsonb_array_elements(r.package->'enhancements') old
                JOIN jsonb_array_elements($2::jsonb) candidate ON candidate->>'path'=old->>'path'
                WHERE r.tenant_id=$1
                  AND EXISTS(SELECT 1 FROM runtime_enhancement_events e WHERE e.tenant_id=r.tenant_id AND e.revision=r.revision)
                  AND (COALESCE(NULLIF(old->'input_schema','null'::jsonb),'true'::jsonb) <> candidate->'input_schema'
                    OR COALESCE(NULLIF(old->'output_schema','null'::jsonb),'true'::jsonb) <> candidate->'output_schema')
            )",
        ).bind(tenant).bind(serde_json::to_value(&contracts)?).fetch_one(&mut *transaction).await?;
        ensure!(
            !conflict,
            "增强路径的 schema 已发布，修改契约必须使用新版本路径"
        );
        let action = match (revision, rollback) {
            (None, _) => "deactivate",
            (_, true) => "rollback",
            _ => "activate",
        };
        sqlx::query("INSERT INTO runtime_enhancement_events(tenant_id,generation,revision,actor,action) VALUES($1,$2,$3,$4,$5)")
            .bind(tenant).bind(next.generation).bind(revision).bind(actor).bind(action).execute(&mut *transaction).await?;
        // 最后一次校验放在 CAS 和行锁等待之后；撤权或取消会回滚整个指针与审计事务。
        authorize.await?;
        transaction.commit().await?;
        Ok(Some(next))
    }

    async fn started(
        &self,
        tenant: &str,
        revision: &str,
        generation: i64,
        path: &str,
    ) -> Result<()> {
        observations::started(&self.pool, tenant, revision, generation, path).await
    }

    async fn finished(
        &self,
        tenant: &str,
        generation: i64,
        path: &str,
        success: bool,
    ) -> Result<()> {
        observations::finished(&self.pool, tenant, generation, path, success).await
    }
}
