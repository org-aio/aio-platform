use super::Components;
use anyhow::Result;
use uuid::Uuid;

impl Components {
    pub async fn rollout(&self) -> Result<()> {
        sqlx::query("INSERT INTO component_rollouts(tenant_id,source_id,digest) SELECT i.tenant_id,i.source_id,p.digest FROM component_installations i JOIN component_publications p ON p.source_id=i.source_id WHERE EXISTS(SELECT 1 FROM component_sources s JOIN delivery_sources d ON d.git=s.git AND d.enabled WHERE s.id=i.source_id) AND i.enabled AND i.digest<>p.digest AND i.excluded_digest IS DISTINCT FROM p.digest ON CONFLICT DO NOTHING")
            .execute(&self.pool).await?;
        let rows = sqlx::query_as::<_, (String, Uuid, String)>("SELECT q.tenant_id,q.source_id,q.digest FROM component_rollouts q JOIN component_installations i ON i.tenant_id=q.tenant_id AND i.source_id=q.source_id JOIN component_publications p ON p.source_id=q.source_id AND p.digest=q.digest WHERE EXISTS(SELECT 1 FROM component_sources s JOIN delivery_sources d ON d.git=s.git AND d.enabled WHERE s.id=i.source_id) AND i.enabled AND i.digest<>q.digest AND i.excluded_digest IS DISTINCT FROM q.digest AND (q.state='queued' OR (q.state='failed' AND q.updated_at<now()-interval '1 minute')) ORDER BY q.updated_at LIMIT 8")
            .fetch_all(&self.pool).await?;
        for (tenant, source, digest) in rows {
            let result = self.upgrade(&tenant, source, &digest).await;
            let status = match &result {
                Ok(true) => "active",
                Ok(false) => "superseded",
                Err(_) => "failed",
            };
            sqlx::query("UPDATE component_rollouts SET state=$4,error=$5,updated_at=now() WHERE tenant_id=$1 AND source_id=$2 AND digest=$3")
                .bind(&tenant).bind(source).bind(&digest).bind(status).bind(result.err().map(|e| format!("{e:#}").chars().take(16000).collect::<String>())).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub(super) async fn upgrade(&self, tenant: &str, source: Uuid, digest: &str) -> Result<bool> {
        let _guard = self.mutations.lock().await;
        // 与启停、卸载、回滚共用锁，领取任务后再次核对租户的最新选择。
        let git: Option<String> = sqlx::query_scalar("SELECT s.git FROM component_installations i JOIN component_sources s ON s.id=i.source_id JOIN component_publications p ON p.source_id=i.source_id WHERE EXISTS(SELECT 1 FROM delivery_sources d WHERE d.git=s.git AND d.enabled) AND i.tenant_id=$1 AND i.source_id=$2 AND i.enabled AND p.digest=$3 AND i.digest<>$3 AND i.excluded_digest IS DISTINCT FROM $3")
            .bind(tenant).bind(source).bind(digest).fetch_optional(&self.pool).await?;
        let Some(git) = git else {
            return Ok(false);
        };
        self.install_locked(tenant, &git, Some(digest), None)
            .await?;
        Ok(true)
    }
}
