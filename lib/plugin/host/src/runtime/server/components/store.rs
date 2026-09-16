use super::{Components, model::Description};
use anyhow::{Context, Result, ensure};
use az_plugin_bundle::Bundle;
use sqlx::Row;
use uuid::Uuid;

impl Components {
    pub async fn published(
        &self,
        git: &str,
        revision: Option<&str>,
    ) -> Result<Option<(Uuid, Bundle)>> {
        let row=sqlx::query("SELECT s.id,v.archive FROM component_sources s JOIN component_publications p ON p.source_id=s.id JOIN component_versions v ON v.source_id=s.id AND v.digest=COALESCE($2,p.digest) WHERE s.git=$1")
            .bind(git).bind(revision).fetch_optional(&self.pool).await?;
        row.map(|r| {
            Ok((
                r.try_get("id")?,
                Bundle::decode(&r.try_get::<Vec<u8>, _>("archive")?)?,
            ))
        })
        .transpose()
    }

    pub async fn contains_source(&self, source: &str) -> Result<bool> {
        let Ok(source) = Uuid::parse_str(source) else {
            return Ok(false);
        };
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM component_publications WHERE source_id=$1)",
        )
        .bind(source)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn publish(&self, bundle: Bundle, readme: &str) -> Result<Uuid> {
        self.publish_checked(bundle, readme, None).await
    }

    pub async fn publish_delivery(&self, bundle: Bundle, readme: &str, job: i64) -> Result<Uuid> {
        self.publish_checked(bundle, readme, Some(job)).await
    }

    async fn publish_checked(
        &self,
        bundle: Bundle,
        readme: &str,
        job: Option<i64>,
    ) -> Result<Uuid> {
        let _guard = self.mutations.lock().await;
        ensure!(readme.len() <= 512 * 1024, "README 超过限制");
        let verified = bundle.verify()?;
        let metadata = verified
            .manifest()
            .plugin
            .marketplace
            .as_ref()
            .context("发布包必须声明 plugin.marketplace")?;
        ensure!(
            metadata.parent.as_deref() != Some(&bundle.git),
            "插件不能以自身为父插件"
        );
        let existing = sqlx::query_as::<_, (Uuid, Option<String>)>(
            "SELECT id,parent_git FROM component_sources WHERE git=$1",
        )
        .bind(&bundle.git)
        .fetch_optional(&self.pool)
        .await?;
        if let Some((_, parent)) = &existing {
            ensure!(parent == &metadata.parent, "已发布来源不能改变父插件归属");
        }
        let source = existing.map(|r| r.0).unwrap_or_else(Uuid::new_v4);
        let previous: Option<bool> = sqlx::query_scalar("SELECT COALESCE((v.description->>'process')::boolean,false) FROM component_publications p JOIN component_versions v ON v.digest=p.digest WHERE p.source_id=$1")
            .bind(source).fetch_optional(&self.pool).await?;
        if let Some(previous) = previous {
            ensure!(
                previous == verified.manifest().plugin.runtime.process.is_some(),
                "已发布来源不能改变运行时类型"
            );
        }
        if let Some(parent) = &metadata.parent {
            let cycle:bool=sqlx::query_scalar("WITH RECURSIVE ancestors AS (SELECT git,parent_git FROM component_sources WHERE git=$1 UNION SELECT s.git,s.parent_git FROM component_sources s JOIN ancestors a ON s.git=a.parent_git) SELECT EXISTS(SELECT 1 FROM ancestors WHERE git=$2 OR parent_git=$2)").bind(parent).bind(&bundle.git).fetch_one(&self.pool).await?;
            ensure!(!cycle, "父子插件关系形成循环");
        }
        let description = self.validate(source, &bundle).await?;
        let archive = bundle.encode()?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO component_sources(id,git,parent_git) VALUES($1,$2,$3) ON CONFLICT(git) DO NOTHING").bind(source).bind(&bundle.git).bind(&metadata.parent).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO component_versions(digest,source_id,archive,version,source_commit,description,metadata,readme,capabilities,permissions) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) ON CONFLICT(digest) DO NOTHING")
            .bind(&bundle.digest).bind(source).bind(archive).bind(&bundle.version).bind(&bundle.commit).bind(serde_json::to_value(description)?).bind(serde_json::to_value(metadata)?).bind(readme).bind(serde_json::to_value(&verified.manifest().plugin.capabilities)?).bind(&verified.manifest().plugin.permissions).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO component_publications(source_id,digest) VALUES($1,$2) ON CONFLICT(source_id) DO UPDATE SET digest=EXCLUDED.digest,published_at=now()").bind(source).bind(&bundle.digest).execute(&mut *tx).await?;
        if let Some(job) = job {
            super::super::delivery::finish_publication(&mut tx, job, &bundle).await?;
        }
        tx.commit().await?;
        Ok(source)
    }

    pub async fn require_parent(&self, tenant: &str, source: Uuid) -> Result<()> {
        let parent: Option<String> =
            sqlx::query_scalar("SELECT parent_git FROM component_sources WHERE id=$1")
                .bind(source)
                .fetch_one(&self.pool)
                .await?;
        if let Some(parent) = parent {
            let installed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM component_installations i JOIN component_sources s ON s.id=i.source_id WHERE i.tenant_id=$1 AND s.git=$2 AND i.enabled) OR EXISTS(SELECT 1 FROM tenant_plugin_bindings i JOIN plugin_sources s ON s.id=i.source_id WHERE i.tenant_id=$1 AND s.git=$2 AND i.enabled)").bind(tenant).bind(&parent).fetch_one(&self.pool).await?;
            ensure!(installed, "请先安装并启用父插件: {parent}");
        }
        Ok(())
    }

    pub async fn require_no_children(&self, tenant: &str, source: Uuid) -> Result<()> {
        let children:Vec<String>=sqlx::query_scalar("SELECT v.metadata->>'title' FROM component_installations i JOIN component_sources s ON s.id=i.source_id JOIN component_versions v ON v.digest=i.digest WHERE i.tenant_id=$1 AND s.parent_git=COALESCE((SELECT git FROM component_sources WHERE id=$2),(SELECT git FROM plugin_sources WHERE id=$2::TEXT))").bind(tenant).bind(source).fetch_all(&self.pool).await?;
        ensure!(
            children.is_empty(),
            "请先卸载子插件: {}",
            children.join("、")
        );
        Ok(())
    }

    pub async fn details(&self, revision: &str) -> Result<Option<serde_json::Value>> {
        let row = sqlx::query(
            "SELECT source_id,version,source_commit,readme FROM component_versions WHERE digest=$1",
        )
        .bind(revision)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let versions=sqlx::query("SELECT digest,version,source_commit,created_at::TEXT FROM component_versions WHERE source_id=$1 ORDER BY created_at DESC LIMIT 20").bind(row.try_get::<Uuid,_>("source_id")?).fetch_all(&self.pool).await?;
        let versions=versions.into_iter().map(|v|Ok::<_,sqlx::Error>(serde_json::json!({"revision":v.try_get::<String,_>("digest")?,"version":v.try_get::<String,_>("version")?,"source_revision":v.try_get::<String,_>("source_commit")?,"created_at":v.try_get::<String,_>("created_at")?}))).collect::<Result<Vec<_>,_>>()?;
        Ok(Some(
            serde_json::json!({"version":row.try_get::<String,_>("version")?,"source_revision":row.try_get::<String,_>("source_commit")?,"readme":row.try_get::<String,_>("readme")?,"versions":versions,"builds":[]}),
        ))
    }

    pub async fn page_count(&self, tenant: &str, source: Uuid) -> Result<usize> {
        Ok(self.description(tenant, source).await?.2.pages.len())
    }

    pub(super) async fn description(
        &self,
        tenant: &str,
        source: Uuid,
    ) -> Result<(String, String, Description)> {
        if tenant == "development"
            && let Some(instance) = self.development.read().await.get(&source)
        {
            return Ok((
                instance.bundle.digest().into(),
                instance.generation.clone(),
                instance.description.clone(),
            ));
        }
        let row=sqlx::query("SELECT i.digest,i.generation::TEXT,v.description FROM component_installations i JOIN component_versions v ON v.digest=i.digest WHERE i.source_id=$1 AND i.tenant_id=$2 AND i.enabled").bind(source).bind(tenant).fetch_optional(&self.pool).await?.context("当前租户未启用插件")?;
        Ok((
            row.try_get("digest")?,
            row.try_get("generation")?,
            serde_json::from_value(row.try_get("description")?)?,
        ))
    }
}
