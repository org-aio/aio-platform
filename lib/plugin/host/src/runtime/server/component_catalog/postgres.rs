use anyhow::{Context, Result, ensure};
use az_plugin_bundle::Bundle;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::{postgres_rows, *};

/// 正式目录实现沿用已有 PostgreSQL 表；连接池由数据库插件拥有和关闭。
pub struct PostgresComponentCatalog {
    pool: PgPool,
}

impl PostgresComponentCatalog {
    /// 默认提供者在初始化阶段调用，迁移或索引损坏会阻止本代启动。
    pub async fn open(pool: PgPool) -> Result<Self> {
        sqlx::raw_sql(include_str!("schema.sql"))
            .execute(&pool)
            .await?;
        postgres_rows::backfill(&pool).await?;
        Ok(Self { pool })
    }

    async fn installed_entries(
        &self,
        tenant: Option<&str>,
        source: Option<Uuid>,
        enabled_only: bool,
    ) -> Result<Vec<ComponentCatalogEntry>> {
        let rows = sqlx::query("SELECT s.id AS source_id,s.git,s.parent_git,coalesce(parent.id,s.id) AS menu_source,v.digest,v.description,v.metadata,v.capabilities,v.permissions,v.required_permissions,i.tenant_id,i.digest AS installed_digest,i.generation,i.enabled,i.excluded_digest FROM component_installations i JOIN component_sources s ON s.id=i.source_id LEFT JOIN component_sources parent ON parent.git=s.parent_git JOIN component_versions v ON v.digest=i.digest WHERE ($1::TEXT IS NULL OR i.tenant_id=$1) AND ($2::UUID IS NULL OR i.source_id=$2) AND (NOT $3 OR i.enabled) ORDER BY i.tenant_id,i.source_id")
            .bind(tenant).bind(source).bind(enabled_only).fetch_all(&self.pool).await?;
        rows.into_iter().map(postgres_rows::entry).collect()
    }
}

#[async_trait::async_trait]
impl ComponentCatalog for PostgresComponentCatalog {
    async fn source(&self, git: &str) -> Result<Option<ComponentSource>> {
        let row = sqlx::query_as::<_, (Uuid, Option<String>)>(
            "SELECT id,parent_git FROM component_sources WHERE git=$1",
        )
        .bind(git)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(id, parent)| ComponentSource {
            id,
            git: git.into(),
            parent,
        }))
    }

    async fn published(&self, git: &str, revision: Option<&str>) -> Result<Option<(Uuid, Bundle)>> {
        let row = sqlx::query_as::<_, (Uuid, Vec<u8>)>("SELECT s.id,v.archive FROM component_sources s JOIN component_publications p ON p.source_id=s.id JOIN component_versions v ON v.source_id=s.id AND v.digest=COALESCE($2,p.digest) WHERE s.git=$1")
            .bind(git).bind(revision).fetch_optional(&self.pool).await?;
        row.map(|(source, bytes)| Ok((source, Bundle::decode(&bytes)?)))
            .transpose()
    }

    async fn contains_source(&self, source: Uuid) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM component_publications WHERE source_id=$1)",
        )
        .bind(source)
        .fetch_one(&self.pool)
        .await?)
    }

    async fn publish(&self, publication: ComponentPublication) -> Result<()> {
        let ComponentPublication {
            source,
            bundle,
            description,
            readme,
            delivery_job,
            replace_publication,
        } = publication;
        let verified = bundle.verify()?;
        let manifest = &verified.manifest().plugin;
        let metadata = manifest
            .marketplace
            .as_ref()
            .context("发布包必须声明 plugin.marketplace")?;
        let archive = bundle.encode()?;
        let mut tx = self.pool.begin().await?;
        // 来源和父子关系跨宿主串行提交，防止两个并发发布分别通过循环检查。
        sqlx::query(
            "SELECT pg_advisory_xact_lock(hashtextextended('aio-component-publication',0))",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO component_sources(id,git,parent_git) VALUES($1,$2,$3) ON CONFLICT(git) DO NOTHING")
            .bind(source).bind(&bundle.git).bind(&metadata.parent).execute(&mut *tx).await?;
        let existing: (Uuid, Option<String>) =
            sqlx::query_as("SELECT id,parent_git FROM component_sources WHERE git=$1 FOR UPDATE")
                .bind(&bundle.git)
                .fetch_one(&mut *tx)
                .await?;
        ensure!(existing.0 == source, "发布来源已变化，请重新验证候选");
        ensure!(
            existing.1 == metadata.parent,
            "已发布来源不能改变父插件归属"
        );
        if let Some(parent) = &metadata.parent {
            let cycle: bool = sqlx::query_scalar("WITH RECURSIVE ancestors AS (SELECT git,parent_git FROM component_sources WHERE git=$1 UNION SELECT s.git,s.parent_git FROM component_sources s JOIN ancestors a ON s.git=a.parent_git) SELECT EXISTS(SELECT 1 FROM ancestors WHERE git=$2 OR parent_git=$2)")
                .bind(parent).bind(&bundle.git).fetch_one(&mut *tx).await?;
            ensure!(!cycle, "父子插件关系形成循环");
        }
        sqlx::query("INSERT INTO component_versions(digest,source_id,archive,version,source_commit,description,metadata,readme,capabilities,permissions,required_permissions) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) ON CONFLICT(digest) DO NOTHING")
            .bind(&bundle.digest).bind(source).bind(archive).bind(&bundle.version).bind(&bundle.commit).bind(serde_json::to_value(description)?).bind(serde_json::to_value(metadata)?).bind(readme).bind(serde_json::to_value(&manifest.capabilities)?).bind(&manifest.permissions).bind(&manifest.required_permissions).execute(&mut *tx).await?;
        let owner: Uuid =
            sqlx::query_scalar("SELECT source_id FROM component_versions WHERE digest=$1")
                .bind(&bundle.digest)
                .fetch_one(&mut *tx)
                .await?;
        ensure!(owner == source, "发布版本不属于当前来源");
        sqlx::query("INSERT INTO component_publications(source_id,digest) VALUES($1,$2) ON CONFLICT(source_id) DO UPDATE SET digest=EXCLUDED.digest,published_at=now() WHERE $3")
            .bind(source).bind(&bundle.digest).bind(replace_publication).execute(&mut *tx).await?;
        if let Some(job) = delivery_job {
            super::super::delivery::finish_publication(&mut tx, job, &bundle).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn entries(&self, tenant: &str) -> Result<Vec<ComponentCatalogEntry>> {
        let rows = sqlx::query("SELECT s.id AS source_id,s.git,s.parent_git,coalesce(parent.id,s.id) AS menu_source,v.digest,v.description,v.metadata,v.capabilities,v.permissions,v.required_permissions,i.tenant_id,i.digest AS installed_digest,i.generation,i.enabled,i.excluded_digest FROM component_sources s LEFT JOIN component_sources parent ON parent.git=s.parent_git LEFT JOIN component_publications p ON p.source_id=s.id LEFT JOIN component_installations i ON i.source_id=s.id AND i.tenant_id=$1 JOIN component_versions v ON v.digest=coalesce(p.digest,i.digest) WHERE p.digest IS NOT NULL OR i.digest IS NOT NULL ORDER BY v.metadata->>'title'")
            .bind(tenant).fetch_all(&self.pool).await?;
        rows.into_iter().map(postgres_rows::entry).collect()
    }

    async fn installations(
        &self,
        tenant: Option<&str>,
        enabled_only: bool,
    ) -> Result<Vec<ComponentCatalogEntry>> {
        self.installed_entries(tenant, None, enabled_only).await
    }

    async fn installation(
        &self,
        tenant: &str,
        source: Uuid,
    ) -> Result<Option<ComponentCatalogEntry>> {
        Ok(self
            .installed_entries(Some(tenant), Some(source), false)
            .await?
            .pop())
    }

    async fn install(&self, commit: ComponentInstallationCommit) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let lock =
            serde_json::to_string(&("component-installation", &commit.tenant, commit.source))?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(lock)
            .execute(&mut *tx)
            .await?;
        let generation: Option<Uuid> = sqlx::query_scalar("SELECT generation FROM component_installations WHERE tenant_id=$1 AND source_id=$2 FOR UPDATE")
            .bind(&commit.tenant).bind(commit.source).fetch_optional(&mut *tx).await?;
        ensure!(
            generation == commit.expected_generation,
            "插件安装代数已变化，拒绝覆盖并发操作"
        );
        let belongs: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM component_versions WHERE source_id=$1 AND digest=$2)",
        )
        .bind(commit.source)
        .bind(&commit.digest)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(belongs, "安装版本不属于当前来源");
        sqlx::query("INSERT INTO component_installations(tenant_id,source_id,digest,generation,excluded_digest) VALUES($1,$2,$3,$4,$5) ON CONFLICT(tenant_id,source_id) DO UPDATE SET digest=EXCLUDED.digest,enabled=true,generation=EXCLUDED.generation,excluded_digest=EXCLUDED.excluded_digest")
            .bind(&commit.tenant).bind(commit.source).bind(&commit.digest).bind(Uuid::new_v4()).bind(&commit.excluded_digest).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO component_initializations(tenant_id,git) SELECT $1,git FROM component_sources WHERE id=$2 ON CONFLICT DO NOTHING")
            .bind(&commit.tenant).bind(commit.source).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn remove(
        &self,
        tenant: &str,
        source: Uuid,
        generation: Uuid,
        uninstall: bool,
    ) -> Result<()> {
        let sql = if uninstall {
            "DELETE FROM component_installations WHERE tenant_id=$1 AND source_id=$2 AND generation=$3"
        } else {
            "UPDATE component_installations SET enabled=false,generation=gen_random_uuid() WHERE tenant_id=$1 AND source_id=$2 AND generation=$3"
        };
        let rows = sqlx::query(sql)
            .bind(tenant)
            .bind(source)
            .bind(generation)
            .execute(&self.pool)
            .await?
            .rows_affected();
        ensure!(rows == 1, "插件安装代数已变化，拒绝覆盖并发操作");
        Ok(())
    }

    async fn rollback_target(&self, tenant: &str, source: Uuid) -> Result<ComponentSelection> {
        let row: (String, String, String) = sqlx::query_as("SELECT s.git,v.digest,p.digest FROM component_versions v JOIN component_sources s ON s.id=v.source_id JOIN component_installations i ON i.source_id=s.id AND i.tenant_id=$2 JOIN component_versions current ON current.digest=i.digest JOIN component_publications p ON p.source_id=s.id WHERE s.id=$1 AND v.created_at<current.created_at ORDER BY v.created_at DESC LIMIT 1")
            .bind(source).bind(tenant).fetch_optional(&self.pool).await?.context("没有可回滚的发布版本")?;
        Ok(ComponentSelection {
            git: row.0,
            digest: row.1,
            excluded_digest: Some(row.2),
        })
    }

    async fn parent_enabled(&self, tenant: &str, parent: &str) -> Result<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM component_installations i JOIN component_sources s ON s.id=i.source_id WHERE i.tenant_id=$1 AND s.git=$2 AND i.enabled) OR EXISTS(SELECT 1 FROM tenant_plugin_bindings i JOIN plugin_sources s ON s.id=i.source_id WHERE i.tenant_id=$1 AND s.git=$2 AND i.enabled)")
            .bind(tenant).bind(parent).fetch_one(&self.pool).await?)
    }

    async fn children(&self, tenant: &str, source: Uuid) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar("SELECT v.metadata->>'title' FROM component_installations i JOIN component_sources s ON s.id=i.source_id JOIN component_versions v ON v.digest=i.digest WHERE i.tenant_id=$1 AND s.parent_git=COALESCE((SELECT git FROM component_sources WHERE id=$2),(SELECT git FROM plugin_sources WHERE id=$2::TEXT))")
            .bind(tenant).bind(source).fetch_all(&self.pool).await?)
    }

    async fn initialized(&self, tenant: &str, git: &str) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM component_initializations WHERE tenant_id=$1 AND git=$2)",
        )
        .bind(tenant)
        .bind(git)
        .fetch_one(&self.pool)
        .await?)
    }

    async fn details(&self, revision: &str) -> Result<Option<serde_json::Value>> {
        let row = sqlx::query(
            "SELECT source_id,version,source_commit,readme FROM component_versions WHERE digest=$1",
        )
        .bind(revision)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let versions = sqlx::query("SELECT digest,version,source_commit,created_at::TEXT FROM component_versions WHERE source_id=$1 ORDER BY created_at DESC LIMIT 20")
            .bind(row.try_get::<Uuid,_>("source_id")?).fetch_all(&self.pool).await?;
        let versions = versions.into_iter().map(|v| Ok::<_,sqlx::Error>(serde_json::json!({"revision":v.try_get::<String,_>("digest")?,"version":v.try_get::<String,_>("version")?,"source_revision":v.try_get::<String,_>("source_commit")?,"created_at":v.try_get::<String,_>("created_at")?}))).collect::<Result<Vec<_>,_>>()?;
        Ok(Some(
            serde_json::json!({"version":row.try_get::<String,_>("version")?,"source_revision":row.try_get::<String,_>("source_commit")?,"readme":row.try_get::<String,_>("readme")?,"versions":versions,"builds":[]}),
        ))
    }

    async fn archive(&self, revision: &str) -> Result<Option<Vec<u8>>> {
        Ok(
            sqlx::query_scalar("SELECT archive FROM component_versions WHERE digest=$1")
                .bind(revision)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    async fn releases(&self, git: &str) -> Result<Vec<Bundle>> {
        let archives: Vec<Vec<u8>> = sqlx::query_scalar("SELECT v.archive FROM component_versions v JOIN component_sources s ON s.id=v.source_id JOIN component_publications p ON p.source_id=s.id WHERE s.git=$1 ORDER BY v.created_at DESC LIMIT 100")
            .bind(git).fetch_all(&self.pool).await?;
        archives
            .into_iter()
            .map(|archive| Bundle::decode(&archive))
            .collect()
    }

    async fn documentation_source(&self, revision: &str) -> Result<String> {
        sqlx::query_scalar("SELECT s.git FROM component_sources s JOIN component_versions v ON v.source_id=s.id WHERE v.digest=$1")
            .bind(revision).fetch_optional(&self.pool).await?.context("发布版本不存在")
    }

    async fn save_documentation(&self, revision: &str, readme: &str) -> Result<()> {
        ensure!(readme.len() <= 512 * 1024, "README 超过限制");
        sqlx::query("UPDATE component_versions SET readme=$2 WHERE digest=$1 AND readme=''")
            .bind(revision)
            .bind(readme)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn process_key(&self, source: Uuid, tenant: &str, candidate: Vec<u8>) -> Result<Vec<u8>> {
        sqlx::query("INSERT INTO component_process_keys(source_id,tenant_id,ciphertext) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
            .bind(source).bind(tenant).bind(candidate).execute(&self.pool).await?;
        Ok(sqlx::query_scalar(
            "SELECT ciphertext FROM component_process_keys WHERE source_id=$1 AND tenant_id=$2",
        )
        .bind(source)
        .bind(tenant)
        .fetch_one(&self.pool)
        .await?)
    }

    async fn record_process_actor(&self, source: Uuid, tenant: &str, user: &str) -> Result<()> {
        sqlx::query("INSERT INTO component_process_actors(source_id,tenant_id,user_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
            .bind(source).bind(tenant).bind(user).execute(&self.pool).await?;
        Ok(())
    }

    async fn process_actor_exists(&self, source: Uuid, tenant: &str, user: &str) -> Result<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM component_process_actors WHERE source_id=$1 AND tenant_id=$2 AND user_id=$3)")
            .bind(source).bind(tenant).bind(user).fetch_one(&self.pool).await?)
    }

    async fn rollouts(&self) -> Result<Vec<ComponentRollout>> {
        sqlx::query("INSERT INTO component_rollouts(tenant_id,source_id,digest) SELECT i.tenant_id,i.source_id,p.digest FROM component_installations i JOIN component_publications p ON p.source_id=i.source_id WHERE EXISTS(SELECT 1 FROM component_sources s JOIN delivery_sources d ON d.git=s.git AND d.enabled WHERE s.id=i.source_id) AND i.enabled AND i.digest<>p.digest AND i.excluded_digest IS DISTINCT FROM p.digest ON CONFLICT DO NOTHING")
            .execute(&self.pool).await?;
        let rows = sqlx::query_as::<_, (String, Uuid, String, String)>("SELECT q.tenant_id,q.source_id,q.digest,i.digest FROM component_rollouts q JOIN component_installations i ON i.tenant_id=q.tenant_id AND i.source_id=q.source_id JOIN component_publications p ON p.source_id=q.source_id AND p.digest=q.digest WHERE EXISTS(SELECT 1 FROM component_sources s JOIN delivery_sources d ON d.git=s.git AND d.enabled WHERE s.id=i.source_id) AND i.enabled AND i.digest<>q.digest AND i.excluded_digest IS DISTINCT FROM q.digest AND (q.state='queued' OR (q.state='failed' AND q.updated_at<now()-interval '1 minute')) ORDER BY q.updated_at")
            .fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(
                |(tenant, source, digest, installed_digest)| ComponentRollout {
                    tenant,
                    source,
                    digest,
                    installed_digest,
                },
            )
            .collect())
    }

    async fn upgrade_target(
        &self,
        tenant: &str,
        source: Uuid,
        digest: &str,
    ) -> Result<Option<String>> {
        Ok(sqlx::query_scalar("SELECT s.git FROM component_installations i JOIN component_sources s ON s.id=i.source_id JOIN component_publications p ON p.source_id=i.source_id WHERE EXISTS(SELECT 1 FROM delivery_sources d WHERE d.git=s.git AND d.enabled) AND i.tenant_id=$1 AND i.source_id=$2 AND i.enabled AND p.digest=$3 AND i.digest<>$3 AND i.excluded_digest IS DISTINCT FROM $3")
            .bind(tenant).bind(source).bind(digest).fetch_optional(&self.pool).await?)
    }

    async fn finish_rollout(
        &self,
        rollout: &ComponentRollout,
        result: ComponentRolloutResult,
    ) -> Result<()> {
        let (status, error) = match result {
            ComponentRolloutResult::Active => ("active", None),
            ComponentRolloutResult::Superseded => ("superseded", None),
            ComponentRolloutResult::Failed(error) => (
                "failed",
                Some(error.chars().take(16000).collect::<String>()),
            ),
        };
        sqlx::query("UPDATE component_rollouts SET state=$4,error=$5,updated_at=now() WHERE tenant_id=$1 AND source_id=$2 AND digest=$3")
            .bind(&rollout.tenant).bind(rollout.source).bind(&rollout.digest).bind(status).bind(error).execute(&self.pool).await?;
        Ok(())
    }
}
