use super::super::component_catalog::ComponentRolloutResult;
use super::Components;
use anyhow::Result;
use uuid::Uuid;

impl Components {
    pub async fn rollout(&self) -> Result<()> {
        let rows = self.catalog.rollouts().await?;
        let mut attempted = 0;
        for rollout in rows {
            // 暂停不改变租户版本，也不消耗升级队列；恢复后仍核对回滚排除记录。
            if !self
                .is_active(rollout.source, &rollout.tenant, &rollout.installed_digest)
                .await
            {
                continue;
            }
            if attempted == 8 {
                break;
            }
            attempted += 1;
            let result = match self
                .upgrade(&rollout.tenant, rollout.source, &rollout.digest)
                .await
            {
                Ok(true) => ComponentRolloutResult::Active,
                Ok(false) => ComponentRolloutResult::Superseded,
                Err(error) => ComponentRolloutResult::Failed(format!("{error:#}")),
            };
            self.catalog.finish_rollout(&rollout, result).await?;
        }
        Ok(())
    }

    pub(super) async fn upgrade(&self, tenant: &str, source: Uuid, digest: &str) -> Result<bool> {
        let _guard = self.mutations.lock().await;
        // 与启停、卸载、回滚共用锁，领取任务后再次核对租户的最新选择。
        let git = self.catalog.upgrade_target(tenant, source, digest).await?;
        let Some(git) = git else {
            return Ok(false);
        };
        self.install_locked(tenant, &git, Some(digest), None)
            .await?;
        Ok(true)
    }
}
