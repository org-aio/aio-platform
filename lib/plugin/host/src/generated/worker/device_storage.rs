use super::*;

impl WorkerServiceImpl {
    pub(super) async fn local_capability(
        &self,
        device: &DeviceIdentity,
        capability: &str,
        enabled: bool,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let existing: Option<serde_json::Value> = sqlx::query_scalar("SELECT capabilities FROM worker_devices WHERE id=$1 AND tenant_id=$2 AND user_id=$3 AND state='active' FOR UPDATE")
            .bind(&device.id).bind(&device.tenant).bind(&device.user)
            .fetch_optional(&mut *tx).await?;
        let mut capabilities: Vec<String> =
            serde_json::from_value(existing.context("设备不存在或已撤销")?)?;
        capabilities.retain(|item| item != capability);
        if enabled {
            capabilities.push(capability.into());
        }
        sqlx::query("UPDATE worker_devices SET capabilities=$2 WHERE id=$1")
            .bind(&device.id)
            .bind(serde_json::to_value(capabilities)?)
            .execute(&mut *tx)
            .await?;
        if !enabled {
            sqlx::query("UPDATE worker_tasks SET state='cancelled',lease=NULL,lease_until=NULL,completed_at=now(),error='本机执行授权已关闭' WHERE worker_id=$1 AND capability=$2 AND state IN ('queued','running')")
                .bind(&device.id).bind(capability).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
