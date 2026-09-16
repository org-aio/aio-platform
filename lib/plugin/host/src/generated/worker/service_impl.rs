use super::{model::*, service::WorkerService, util::*};
use crate::identity::SessionContext;
use anyhow::{Context, Result, ensure};
use sqlx::{PgPool, Row};

#[dill::component]
#[dill::interface(dyn WorkerService)]
#[dill::scope(dill::Singleton)]
pub(crate) struct WorkerServiceImpl {
    pool: PgPool,
}

#[async_trait::async_trait]
impl WorkerService for WorkerServiceImpl {
    async fn pair(&self, request: PairRequest) -> Result<Pairing> {
        ensure!(
            !request.label.trim().is_empty()
                && request.label.len() <= 120
                && request.platform.len() <= 80,
            "设备名称无效"
        );
        ensure!(
            !request.capabilities.is_empty() && request.capabilities.len() <= 32,
            "设备能力数量无效"
        );
        for item in &request.capabilities {
            validate_capability(item)?;
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('worker-pairing',0))")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM worker_devices WHERE state='pending' AND expires_at<now()")
            .execute(&mut *tx)
            .await?;
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM worker_devices WHERE state='pending'")
                .fetch_one(&mut *tx)
                .await?;
        ensure!(count < 128, "配对请求繁忙，请稍后重试");
        let id = uuid::Uuid::new_v4().to_string();
        let token = secret()?;
        let code = uuid::Uuid::new_v4().simple().to_string();
        let expires_at=sqlx::query_scalar("INSERT INTO worker_devices(id,token_hash,pairing_code,label,platform,capabilities) VALUES($1,$2,$3,$4,$5,$6) RETURNING (extract(epoch FROM expires_at)*1000)::bigint")
            .bind(&id).bind(digest(&token)).bind(&code).bind(request.label).bind(request.platform).bind(serde_json::to_value(request.capabilities)?).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(Pairing {
            device_id: id,
            code,
            token,
            expires_at,
        })
    }
    async fn pairing(&self, code: &str) -> Result<Worker> {
        let row=sqlx::query("SELECT *,state AS status,NULL::bigint AS last_seen_ms FROM worker_devices WHERE pairing_code=$1 AND state='pending' AND expires_at>now()")
            .bind(code).fetch_optional(&self.pool).await?.context("配对码无效或已过期")?;
        worker(row)
    }
    async fn approve(&self, session: &SessionContext, code: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "worker-owner:{}:{}",
                session.tenant_id, session.user_id
            ))
            .execute(&mut *tx)
            .await?;
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM worker_devices WHERE tenant_id=$1 AND user_id=$2 AND state='active'").bind(&session.tenant_id).bind(&session.user_id).fetch_one(&mut *tx).await?;
        ensure!(count < 32, "当前账号设备数量已达上限");
        let updated=sqlx::query("UPDATE worker_devices SET tenant_id=$1,user_id=$2,state='active',pairing_code=NULL WHERE pairing_code=$3 AND state='pending' AND expires_at>now()")
            .bind(&session.tenant_id).bind(&session.user_id).bind(code).execute(&mut *tx).await?.rows_affected();
        ensure!(updated == 1, "配对码无效、已使用或已过期");
        tx.commit().await?;
        Ok(())
    }
    async fn poll(&self, token: &str) -> Result<String> {
        let state=sqlx::query_scalar("SELECT state FROM worker_devices WHERE token_hash=$1 AND (state<>'pending' OR expires_at>now())")
            .bind(digest(token)).fetch_optional(&self.pool).await?.context("配对请求已过期")?;
        Ok(state)
    }
    async fn identity(&self, token: &str) -> Result<DeviceIdentity> {
        let row=sqlx::query("SELECT id,tenant_id,user_id,capabilities FROM worker_devices WHERE token_hash=$1 AND state='active'")
            .bind(digest(token)).fetch_optional(&self.pool).await?.context("设备尚未授权或已撤销")?;
        Ok(DeviceIdentity {
            id: row.try_get("id")?,
            tenant: row.try_get("tenant_id")?,
            user: row.try_get("user_id")?,
            capabilities: serde_json::from_value(row.try_get("capabilities")?)?,
        })
    }
    async fn list(&self, session: &SessionContext) -> Result<Vec<Worker>> {
        let rows=sqlx::query("SELECT *,CASE WHEN state='active' THEN CASE WHEN last_seen>now()-interval '90 seconds' THEN 'online' ELSE 'offline' END ELSE state END AS status,(extract(epoch FROM last_seen)*1000)::bigint AS last_seen_ms FROM worker_devices WHERE tenant_id=$1 AND user_id=$2 ORDER BY created_at DESC LIMIT 100")
            .bind(&session.tenant_id).bind(&session.user_id).fetch_all(&self.pool).await?;
        rows.into_iter().map(worker).collect()
    }
    async fn revoke(&self, session: &SessionContext, id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let n = sqlx::query(
            "UPDATE worker_devices SET state='revoked' WHERE id=$1 AND tenant_id=$2 AND user_id=$3",
        )
        .bind(id)
        .bind(&session.tenant_id)
        .bind(&session.user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        ensure!(n == 1, "设备不存在");
        sqlx::query("UPDATE worker_tasks SET state='cancelled',lease=NULL,lease_until=NULL,error='设备授权已撤销' WHERE worker_id=$1 AND state IN ('queued','running')").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    async fn enqueue(&self, session: &SessionContext, request: SubmitTask) -> Result<Task> {
        uuid::Uuid::parse_str(&request.id)?;
        validate_capability(&request.capability)?;
        ensure!(
            serde_json::to_vec(&request.input)?.len() <= 32_768,
            "任务输入过大"
        );
        let mut tx = self.pool.begin().await?;
        let row=sqlx::query("SELECT capabilities FROM worker_devices WHERE id=$1 AND tenant_id=$2 AND user_id=$3 AND state='active' FOR UPDATE")
            .bind(&request.worker_id).bind(&session.tenant_id).bind(&session.user_id).fetch_optional(&mut *tx).await?.context("设备不存在或已撤销")?;
        let capabilities: Vec<String> = serde_json::from_value(row.try_get("capabilities")?)?;
        ensure!(
            capabilities.contains(&request.capability),
            "设备未声明该任务能力"
        );
        let pending:i64=sqlx::query_scalar("SELECT count(*) FROM worker_tasks WHERE worker_id=$1 AND state IN ('queued','running')").bind(&request.worker_id).fetch_one(&mut *tx).await?;
        ensure!(pending < 100, "设备等待任务过多");
        sqlx::query("INSERT INTO worker_tasks(id,worker_id,tenant_id,user_id,capability,input) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING")
            .bind(&request.id).bind(&request.worker_id).bind(&session.tenant_id).bind(&session.user_id).bind(&request.capability).bind(&request.input).execute(&mut *tx).await?;
        let row=sqlx::query("SELECT *,NULL::text AS lease,(extract(epoch FROM created_at)*1000)::bigint AS created_at_ms FROM worker_tasks WHERE id=$1 AND tenant_id=$2 AND user_id=$3 AND worker_id=$4 AND capability=$5 AND input=$6")
            .bind(&request.id).bind(&session.tenant_id).bind(&session.user_id).bind(&request.worker_id).bind(&request.capability).bind(&request.input).fetch_optional(&mut *tx).await?.context("任务 ID 已被不同请求占用")?;
        let mut value = task(row)?;
        value.lease = None;
        tx.commit().await?;
        Ok(value)
    }
    async fn tasks(&self, session: &SessionContext) -> Result<Vec<Task>> {
        self.expire().await?;
        let rows=sqlx::query("SELECT *,NULL::text AS lease,(extract(epoch FROM created_at)*1000)::bigint AS created_at_ms FROM worker_tasks WHERE tenant_id=$1 AND user_id=$2 ORDER BY created_at DESC LIMIT 100")
            .bind(&session.tenant_id).bind(&session.user_id).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|row| {
                let mut value = task(row)?;
                value.lease = None;
                Ok(value)
            })
            .collect()
    }
    async fn task(&self, session: &SessionContext, id: &str) -> Result<Task> {
        self.expire().await?;
        let row = sqlx::query("SELECT *,NULL::text AS lease,(extract(epoch FROM created_at)*1000)::bigint AS created_at_ms FROM worker_tasks WHERE id=$1 AND tenant_id=$2 AND user_id=$3")
            .bind(id).bind(&session.tenant_id).bind(&session.user_id)
            .fetch_optional(&self.pool).await?.context("任务不存在")?;
        task(row)
    }
    async fn desktop(&self, session: &SessionContext, id: &str, enabled: bool) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let existing: Option<serde_json::Value> = sqlx::query_scalar("SELECT capabilities FROM worker_devices WHERE id=$1 AND tenant_id=$2 AND user_id=$3 AND state='active' AND platform='darwin' FOR UPDATE")
            .bind(id).bind(&session.tenant_id).bind(&session.user_id).fetch_optional(&mut *tx).await?;
        let mut capabilities: Vec<String> =
            serde_json::from_value(existing.context("设备不存在或不支持应用控制")?)?;
        capabilities.retain(|capability| capability != "desktop.open-app");
        if enabled {
            capabilities.push("desktop.open-app".into());
        }
        sqlx::query("UPDATE worker_devices SET capabilities=$2 WHERE id=$1")
            .bind(id)
            .bind(serde_json::to_value(capabilities)?)
            .execute(&mut *tx)
            .await?;
        if !enabled {
            sqlx::query("UPDATE worker_tasks SET state='cancelled',lease=NULL,lease_until=NULL,error='应用控制授权已关闭' WHERE worker_id=$1 AND capability='desktop.open-app' AND state IN ('queued','running')")
                .bind(id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    async fn claim(&self, device: &DeviceIdentity, request_id: &str) -> Result<Option<Task>> {
        self.expire().await?;
        let mut tx = self.pool.begin().await?;
        let active: Option<String> = sqlx::query_scalar(
            "SELECT id FROM worker_devices WHERE id=$1 AND state='active' FOR UPDATE",
        )
        .bind(&device.id)
        .fetch_optional(&mut *tx)
        .await?;
        ensure!(active.is_some(), "设备已撤销");
        let previous = sqlx::query("SELECT *,lease_until>now() AS valid_lease,(extract(epoch FROM created_at)*1000)::bigint AS created_at_ms FROM worker_tasks WHERE worker_id=$1 AND claim_id=$2")
            .bind(&device.id).bind(request_id).fetch_optional(&mut *tx).await?;
        if let Some(row) = previous {
            let running = row.try_get::<String, _>("state")? == "running";
            let valid = row
                .try_get::<Option<bool>, _>("valid_lease")?
                .unwrap_or(false);
            return if running && valid {
                Ok(Some(task(row)?))
            } else {
                Ok(None)
            };
        }
        let busy: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM worker_tasks WHERE worker_id=$1 AND state='running')",
        )
        .bind(&device.id)
        .fetch_one(&mut *tx)
        .await?;
        if busy {
            return Ok(None);
        }
        let lease = secret()?;
        let row=sqlx::query("WITH next AS (SELECT id FROM worker_tasks WHERE worker_id=$1 AND state='queued' ORDER BY created_at FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE worker_tasks t SET state='running',lease=$2,claim_id=$3,lease_until=now()+interval '2 minutes' FROM next WHERE t.id=next.id RETURNING t.*,(extract(epoch FROM t.created_at)*1000)::bigint AS created_at_ms")
            .bind(&device.id).bind(lease).bind(request_id).fetch_optional(&mut *tx).await?;
        tx.commit().await?;
        row.map(task).transpose()
    }
    async fn heartbeat(&self, device: &DeviceIdentity, id: Option<(&str, &str)>) -> Result<()> {
        let n =
            sqlx::query("UPDATE worker_devices SET last_seen=now() WHERE id=$1 AND state='active'")
                .bind(&device.id)
                .execute(&self.pool)
                .await?
                .rows_affected();
        ensure!(n == 1, "设备已撤销");
        if let Some((id, lease)) = id {
            let n=sqlx::query("UPDATE worker_tasks SET lease_until=now()+interval '2 minutes' WHERE id=$1 AND worker_id=$2 AND state='running' AND lease=$3 AND lease_until>now()")
                .bind(id).bind(&device.id).bind(lease).execute(&self.pool).await?.rows_affected();
            ensure!(n == 1, "任务租约失效");
        }
        Ok(())
    }
    async fn complete(
        &self,
        device: &DeviceIdentity,
        id: &str,
        request: CompleteTask,
    ) -> Result<()> {
        ensure!(
            serde_json::to_vec(&request.result)?.len() <= 512_000
                && request.error.as_ref().is_none_or(|e| e.len() <= 4096),
            "任务结果过大"
        );
        let state = if request.error.is_some() {
            "failed"
        } else {
            "complete"
        };
        let n=sqlx::query("UPDATE worker_tasks SET state=$4,result=$5,error=$6,completed_at=now(),lease=NULL,lease_until=NULL WHERE id=$1 AND worker_id=$2 AND state='running' AND lease=$3 AND lease_until>now() AND EXISTS(SELECT 1 FROM worker_devices WHERE id=$2 AND state='active')")
            .bind(id).bind(&device.id).bind(&request.lease).bind(state).bind(&request.result).bind(&request.error).execute(&self.pool).await?.rows_affected();
        if n == 0 {
            let completed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM worker_tasks WHERE id=$1 AND worker_id=$2 AND state=$3 AND result IS NOT DISTINCT FROM $4 AND error IS NOT DISTINCT FROM $5)")
                .bind(id).bind(&device.id).bind(state).bind(&request.result).bind(&request.error).fetch_one(&self.pool).await?;
            ensure!(completed, "任务已完成或租约失效");
        }
        Ok(())
    }
}
impl WorkerServiceImpl {
    async fn expire(&self) -> Result<()> {
        // 未知是否完成的副作用任务不自动重派，由用户检查结果后重试。
        sqlx::query("UPDATE worker_tasks SET state='interrupted',error='设备租约过期，执行结果待确认',lease=NULL,lease_until=NULL WHERE state='running' AND lease_until<now()")
            .execute(&self.pool).await?;
        Ok(())
    }
}
