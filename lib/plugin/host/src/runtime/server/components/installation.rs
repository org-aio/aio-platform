use super::Components;
use anyhow::{Context, Result, ensure};
use uuid::Uuid;

impl Components {
    pub async fn install(&self, tenant: &str, git: &str, revision: Option<&str>) -> Result<()> {
        let _guard = self.mutations.lock().await;
        self.install_locked(tenant, git, revision, None).await
    }

    pub(super) async fn install_locked(
        &self,
        tenant: &str,
        git: &str,
        revision: Option<&str>,
        excluded: Option<&str>,
    ) -> Result<()> {
        let (source, bundle) = self
            .published(git, revision)
            .await?
            .context("插件尚未发布")?;
        self.require_parent(tenant, source).await?;
        if bundle.verify()?.manifest().plugin.runtime.process.is_some() {
            return self.install_process(tenant, source, bundle, excluded).await;
        }
        let resources = self.resources(source, tenant, &bundle).await?;
        let grants = bundle.verify()?.manifest().plugin.capabilities.clone();
        let slot = self.slot(source, tenant).await?;
        let previous = slot.stored().await?;
        slot.activate(&self.engine, bundle.clone(), grants, resources)
            .await?;
        let result = async {
            let mut tx = self.pool.begin().await?;
            self.save_installation(&mut tx, tenant, source, &bundle.digest, excluded)
                .await?;
            tx.commit().await?;
            anyhow::Ok(())
        }
        .await;
        if let Err(error) = result {
            if let Some(previous) = previous {
                let resources = self.resources(source, tenant, &previous.bundle).await?;
                slot.activate(&self.engine, previous.bundle, previous.grants, resources)
                    .await?;
            } else {
                slot.deactivate().await?;
            }
            return Err(error);
        }
        Ok(())
    }

    pub async fn change(&self, tenant: &str, source: Uuid, action: &str) -> Result<()> {
        let _guard = self.mutations.lock().await;
        if action == "enable" {
            let git:String=sqlx::query_scalar("SELECT s.git FROM component_sources s JOIN component_installations i ON i.source_id=s.id WHERE s.id=$1 AND i.tenant_id=$2").bind(source).bind(tenant).fetch_one(&self.pool).await?;
            return self.install_locked(tenant, &git, None, None).await;
        }
        if action == "rollback" {
            let row=sqlx::query_as::<_,(String,String)>("SELECT s.git,v.digest FROM component_versions v JOIN component_sources s ON s.id=v.source_id JOIN component_installations i ON i.source_id=s.id AND i.tenant_id=$2 JOIN component_versions current ON current.digest=i.digest WHERE s.id=$1 AND v.created_at<current.created_at ORDER BY v.created_at DESC LIMIT 1").bind(source).bind(tenant).fetch_optional(&self.pool).await?.context("没有可回滚的发布版本")?;
            let excluded: String =
                sqlx::query_scalar("SELECT digest FROM component_publications WHERE source_id=$1")
                    .bind(source)
                    .fetch_one(&self.pool)
                    .await?;
            return self
                .install_locked(tenant, &row.0, Some(&row.1), Some(&excluded))
                .await;
        }
        ensure!(action == "disable" || action == "uninstall", "未知管理操作");
        self.require_no_children(tenant, source).await?;
        let installed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM component_installations WHERE source_id=$1 AND tenant_id=$2)").bind(source).bind(tenant).fetch_one(&self.pool).await?;
        ensure!(installed, "当前租户未安装插件");
        if let Some(bundle) = self.installed_bundle(tenant, source).await?
            && bundle.verify()?.manifest().plugin.runtime.process.is_some()
        {
            return self.change_process(tenant, source, action, bundle).await;
        }
        let slot = self.slot(source, tenant).await?;
        let previous = slot.stored().await?;
        slot.deactivate().await?;
        let sql = if action == "uninstall" {
            "DELETE FROM component_installations WHERE source_id=$1 AND tenant_id=$2"
        } else {
            "UPDATE component_installations SET enabled=false,generation=gen_random_uuid() WHERE source_id=$1 AND tenant_id=$2"
        };
        let result = sqlx::query(sql)
            .bind(source)
            .bind(tenant)
            .execute(&self.pool)
            .await;
        if let Err(error) = result {
            if let Some(previous) = previous {
                let resources = self.resources(source, tenant, &previous.bundle).await?;
                slot.activate(&self.engine, previous.bundle, previous.grants, resources)
                    .await?;
            }
            return Err(error.into());
        }
        Ok(())
    }
    pub(super) async fn save_installation(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant: &str,
        source: Uuid,
        digest: &str,
        excluded: Option<&str>,
    ) -> Result<()> {
        sqlx::query("INSERT INTO component_installations(tenant_id,source_id,digest,generation,excluded_digest) VALUES($1,$2,$3,$4,$5) ON CONFLICT(tenant_id,source_id) DO UPDATE SET digest=EXCLUDED.digest,enabled=true,generation=EXCLUDED.generation,excluded_digest=EXCLUDED.excluded_digest")
            .bind(tenant).bind(source).bind(digest).bind(Uuid::new_v4()).bind(excluded).execute(&mut **tx).await?;
        Ok(())
    }
}
