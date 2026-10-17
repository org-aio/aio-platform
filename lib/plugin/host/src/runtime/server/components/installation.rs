use std::sync::Arc;

use super::{Components, execution::ComponentExecutor};
use anyhow::{Context, Result, ensure};
use az_plugin_bundle::Bundle;
use uuid::Uuid;

struct PreviousExecution {
    executor: Arc<dyn ComponentExecutor>,
    bundle: Bundle,
}

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
        let executor = self.executor(&bundle)?;
        let previous = self.previous_execution(tenant, source).await?;
        let result = async {
            executor.activate(source, tenant, &bundle).await?;
            let mut tx = self.pool.begin().await?;
            self.save_installation(&mut tx, tenant, source, &bundle.digest, excluded)
                .await?;
            tx.commit().await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            return match self
                .recover_execution(source, tenant, &executor, previous.as_ref())
                .await
            {
                Ok(()) => Err(error),
                Err(cleanup) => {
                    Err(error.context(format!("安装失败且执行器恢复失败: {cleanup:#}")))
                }
            };
        }
        self.bind_execution(source, tenant, &bundle.digest, executor.clone())
            .await;
        if let Some(previous) = previous
            && previous.executor.as_ref().type_id() != executor.as_ref().type_id()
            && let Err(error) = previous.executor.deactivate(source, tenant).await
        {
            // 安装提交点已经完成，旧实例清理失败不能伪装成新版本未激活。
            eprintln!("清理旧执行器实例失败 ({source}, {tenant}): {error:#}");
        }
        Ok(())
    }

    async fn previous_execution(
        &self,
        tenant: &str,
        source: Uuid,
    ) -> Result<Option<PreviousExecution>> {
        let Some(bundle) = self.enabled_bundle(tenant, source).await? else {
            return Ok(None);
        };
        let active = self.active.read().await;
        let Some(active) = active.get(&(source, tenant.into())) else {
            return Ok(None);
        };
        ensure!(
            active.digest == bundle.digest,
            "插件活动版本与安装记录不匹配"
        );
        let executor = active.executor.clone();
        Ok(Some(PreviousExecution { executor, bundle }))
    }

    async fn recover_execution(
        &self,
        source: Uuid,
        tenant: &str,
        candidate: &Arc<dyn ComponentExecutor>,
        previous: Option<&PreviousExecution>,
    ) -> Result<()> {
        let mut errors = Vec::new();
        if previous.is_none_or(|previous| {
            previous.executor.as_ref().type_id() != candidate.as_ref().type_id()
        }) && let Err(error) = candidate.deactivate(source, tenant).await
        {
            errors.push(format!("清理候选失败: {error:#}"));
        }
        if let Some(previous) = previous
            && let Err(error) = previous
                .executor
                .restore(source, tenant, &previous.bundle)
                .await
        {
            errors.push(format!("恢复旧实例失败: {error:#}"));
        }
        ensure!(errors.is_empty(), "{}", errors.join("; "));
        Ok(())
    }

    pub async fn change(&self, tenant: &str, source: Uuid, action: &str) -> Result<()> {
        let _guard = self.mutations.lock().await;
        if action == "enable" {
            // 重新启用只恢复租户选择的版本，不能撤销回滚或隐式升级。
            let (git, digest, excluded) = sqlx::query_as::<_, (String, String, Option<String>)>(
                "SELECT s.git,i.digest,i.excluded_digest FROM component_sources s JOIN component_installations i ON i.source_id=s.id WHERE s.id=$1 AND i.tenant_id=$2",
            )
            .bind(source)
            .bind(tenant)
            .fetch_one(&self.pool)
            .await?;
            return self
                .install_locked(tenant, &git, Some(&digest), excluded.as_deref())
                .await;
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
        self.installed_bundle(tenant, source)
            .await?
            .context("当前租户未安装插件")?;
        let previous = self.previous_execution(tenant, source).await?;
        let result = async {
            if let Some(previous) = &previous {
                previous.executor.deactivate(source, tenant).await?;
            }
            let sql = if action == "uninstall" {
                "DELETE FROM component_installations WHERE source_id=$1 AND tenant_id=$2"
            } else {
                "UPDATE component_installations SET enabled=false,generation=gen_random_uuid() WHERE source_id=$1 AND tenant_id=$2"
            };
            sqlx::query(sql).bind(source).bind(tenant).execute(&self.pool).await?;
            Ok::<_, anyhow::Error>(())
        }.await;
        if let Err(error) = result {
            let Some(previous) = previous.as_ref() else {
                return Err(error);
            };
            return match self
                .recover_execution(source, tenant, &previous.executor, Some(previous))
                .await
            {
                Ok(()) => Err(error),
                Err(cleanup) => {
                    Err(error.context(format!("停用失败且执行器恢复失败: {cleanup:#}")))
                }
            };
        }
        self.active.write().await.remove(&(source, tenant.into()));
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
        // 与安装共用提交点，失败不能留下已初始化标记，卸载也不清除此记录。
        sqlx::query("INSERT INTO component_initializations(tenant_id,git) SELECT $1,git FROM component_sources WHERE id=$2 ON CONFLICT DO NOTHING")
            .bind(tenant).bind(source).execute(&mut **tx).await?;
        Ok(())
    }
}
