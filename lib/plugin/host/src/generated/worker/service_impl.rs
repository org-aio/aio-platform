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
        let machine_id = match request.machine_id {
            Some(value) => Some(
                uuid::Uuid::parse_str(&value)
                    .context("设备标识无效")?
                    .to_string(),
            ),
            None => None,
        };
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
        let expires_at=sqlx::query_scalar("INSERT INTO worker_devices(id,token_hash,pairing_code,label,platform,capabilities,machine_id) VALUES($1,$2,$3,$4,$5,$6,$7) RETURNING (extract(epoch FROM expires_at)*1000)::bigint")
            .bind(&id).bind(digest(&token)).bind(&code).bind(request.label).bind(request.platform).bind(serde_json::to_value(request.capabilities)?).bind(machine_id).fetch_one(&mut *tx).await?;
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
        let pending=sqlx::query("SELECT id,machine_id FROM worker_devices WHERE pairing_code=$1 AND state='pending' AND expires_at>now() FOR UPDATE")
            .bind(code).fetch_optional(&mut *tx).await?.context("配对码无效、已使用或已过期")?;
        let id: String = pending.try_get("id")?;
        let machine_id: Option<String> = pending.try_get("machine_id")?;
        // 按账号和稳定本机身份替换凭据；同名设备不参与去重，旧任务停止执行。
        if let Some(machine_id) = machine_id {
            // 同一账号重新配对同一机器时保留用户备注，不依据主机名猜测。
            sqlx::query("UPDATE worker_devices SET note=(SELECT note FROM worker_devices WHERE tenant_id=$1 AND user_id=$2 AND machine_id=$3 AND state='active' LIMIT 1) WHERE id=$4")
                .bind(&session.tenant_id).bind(&session.user_id).bind(&machine_id).bind(&id)
                .execute(&mut *tx).await?;
            let stale=sqlx::query_scalar::<_,String>("UPDATE worker_devices SET state='revoked' WHERE tenant_id=$1 AND user_id=$2 AND state='active' AND machine_id=$3 RETURNING id")
                .bind(&session.tenant_id).bind(&session.user_id).bind(machine_id).fetch_all(&mut *tx).await?;
            sqlx::query("UPDATE worker_tasks SET state='cancelled',lease=NULL,lease_until=NULL,completed_at=now(),error='设备重新配对' WHERE worker_id=ANY($1) AND state IN ('queued','running')")
                .bind(stale).execute(&mut *tx).await?;
        }
        // 已达设备上限时，替换已有机器仍然允许；检查失败会回滚撤销操作。
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM worker_devices WHERE tenant_id=$1 AND user_id=$2 AND state='active'").bind(&session.tenant_id).bind(&session.user_id).fetch_one(&mut *tx).await?;
        ensure!(count < 32, "当前账号设备数量已达上限");
        let updated=sqlx::query("UPDATE worker_devices SET tenant_id=$1,user_id=$2,state='active',pairing_code=NULL WHERE id=$3 AND state='pending'")
            .bind(&session.tenant_id).bind(&session.user_id).bind(id).execute(&mut *tx).await?.rows_affected();
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
        let rows=sqlx::query("SELECT *,CASE WHEN last_seen>now()-interval '90 seconds' THEN 'online' ELSE 'offline' END AS status,(extract(epoch FROM last_seen)*1000)::bigint AS last_seen_ms FROM worker_devices WHERE tenant_id=$1 AND user_id=$2 AND state='active' ORDER BY created_at DESC LIMIT 100")
            .bind(&session.tenant_id).bind(&session.user_id).fetch_all(&self.pool).await?;
        rows.into_iter().map(worker).collect()
    }
    async fn note(&self, session: &SessionContext, id: &str, note: String) -> Result<()> {
        let note = validate_device_note(&note)?;
        let updated = sqlx::query("UPDATE worker_devices SET note=NULLIF($4,'') WHERE id=$1 AND tenant_id=$2 AND user_id=$3 AND state='active'")
            .bind(id).bind(&session.tenant_id).bind(&session.user_id).bind(note)
            .execute(&self.pool).await?.rows_affected();
        ensure!(updated == 1, "设备不存在或无权编辑");
        Ok(())
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
        if request.capability == "workspace.execute" {
            validate_workspace_input(&request.input)?;
        }
        if request.capability == "workspace.manage" {
            validate_workspace_input(&request.input)?;
        }
        if request.capability == "desktop.control" {
            validate_desktop_input(&request.input)?;
        }
        if request.capability == "adb.control" {
            validate_adb_input(&request.input)?;
        }
        if request.capability == "clipboard.sync" {
            validate_clipboard_input(&request.input)?;
        }
        let mut tx = self.pool.begin().await?;
        let row=sqlx::query("SELECT capabilities FROM worker_devices WHERE id=$1 AND tenant_id=$2 AND user_id=$3 AND state='active' FOR UPDATE")
            .bind(&request.worker_id).bind(&session.tenant_id).bind(&session.user_id).fetch_optional(&mut *tx).await?.context("设备不存在或已撤销")?;
        let capabilities: Vec<String> = serde_json::from_value(row.try_get("capabilities")?)?;
        ensure!(
            capabilities.contains(&request.capability),
            "设备未声明该任务能力"
        );
        if request.capability == "adb.control"
            && request.input.get("action").and_then(|value| value.as_str()) == Some("shell")
        {
            ensure!(
                capabilities
                    .iter()
                    .any(|capability| capability == "adb.shell"),
                "设备未启用原始 ADB Shell"
            );
        }
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
    async fn cancel_task(&self, session: &SessionContext, id: &str) -> Result<Task> {
        uuid::Uuid::parse_str(id)?;
        let mut tx = self.pool.begin().await?;
        let exists: Option<String> = sqlx::query_scalar(
            "SELECT id FROM worker_tasks WHERE id=$1 AND tenant_id=$2 AND user_id=$3 FOR UPDATE",
        )
        .bind(id)
        .bind(&session.tenant_id)
        .bind(&session.user_id)
        .fetch_optional(&mut *tx)
        .await?;
        ensure!(exists.is_some(), "任务不存在");
        // 与完成回执竞争时由任务行锁决定终态，重复取消不改写已保存结果。
        sqlx::query("UPDATE worker_tasks SET state='cancelled',lease=NULL,lease_until=NULL,completed_at=now(),error='用户已取消任务' WHERE id=$1 AND state IN ('queued','running')")
            .bind(id).execute(&mut *tx).await?;
        let row = sqlx::query("SELECT *,NULL::text AS lease,(extract(epoch FROM created_at)*1000)::bigint AS created_at_ms FROM worker_tasks WHERE id=$1")
            .bind(id).fetch_one(&mut *tx).await?;
        let value = task(row)?;
        tx.commit().await?;
        Ok(value)
    }
    async fn workspace_access(&self, device: &DeviceIdentity, enabled: bool) -> Result<()> {
        self.local_capability(device, "workspace.execute", enabled)
            .await
    }
    async fn desktop_access(&self, device: &DeviceIdentity, enabled: bool) -> Result<()> {
        self.local_capability(device, "desktop.control", enabled)
            .await
    }
    async fn adb_access(&self, device: &DeviceIdentity, enabled: bool) -> Result<()> {
        self.local_capability(device, "adb.control", enabled).await
    }
    async fn adb_shell_access(&self, device: &DeviceIdentity, enabled: bool) -> Result<()> {
        self.local_capability(device, "adb.shell", enabled).await
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
    async fn terminal_create(
        &self,
        session: &SessionContext,
        request: CreateTerminal,
    ) -> Result<TerminalSession> {
        uuid::Uuid::parse_str(&request.worker_id)?;
        validate_terminal_size(request.cols, request.rows)?;
        self.expire_terminals().await?;
        let mut tx = self.pool.begin().await?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM worker_devices WHERE id=$1 AND tenant_id=$2 AND user_id=$3 AND state='active' AND capabilities ? 'terminal.open')",
        )
        .bind(&request.worker_id)
        .bind(&session.tenant_id)
        .bind(&session.user_id)
        .fetch_one(&mut *tx)
        .await?;
        ensure!(exists, "设备不存在或已撤销");
        // 每台设备只保持一个浏览器终端；新页面接管时让旧 PTY 主动退出。
        sqlx::query(
            "UPDATE worker_terminal_sessions SET state='closing',updated_at=now() WHERE worker_id=$1 AND state IN ('waiting','active')",
        )
        .bind(&request.worker_id)
        .execute(&mut *tx)
        .await?;
        let id = uuid::Uuid::new_v4().to_string();
        let row = sqlx::query(
            "INSERT INTO worker_terminal_sessions(id,worker_id,tenant_id,user_id,cols,rows) VALUES($1,$2,$3,$4,$5,$6) RETURNING *, (extract(epoch FROM created_at)*1000)::bigint AS created_at_ms",
        )
        .bind(&id)
        .bind(&request.worker_id)
        .bind(&session.tenant_id)
        .bind(&session.user_id)
        .bind(i32::from(request.cols))
        .bind(i32::from(request.rows))
        .fetch_one(&mut *tx)
        .await?;
        let value = terminal(row)?;
        tx.commit().await?;
        Ok(value)
    }
    async fn terminal_devices(&self, session: &SessionContext) -> Result<Vec<Worker>> {
        // 只有已本机开启终端且仍在线的设备可供浏览器选择。
        let rows = sqlx::query(
            "SELECT *, 'online'::text AS status, (extract(epoch FROM last_seen)*1000)::bigint AS last_seen_ms FROM worker_devices WHERE tenant_id=$1 AND user_id=$2 AND state='active' AND last_seen>now()-interval '90 seconds' AND capabilities ? 'terminal.open' ORDER BY created_at DESC LIMIT 100",
        )
        .bind(&session.tenant_id)
        .bind(&session.user_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(worker).collect()
    }
    async fn terminal_events(
        &self,
        session: &SessionContext,
        id: &str,
        after: u64,
        wait_seconds: u8,
    ) -> Result<TerminalEvents> {
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(u64::from(wait_seconds));
        loop {
            let row = sqlx::query(
                "SELECT state FROM worker_terminal_sessions WHERE id=$1 AND tenant_id=$2 AND user_id=$3",
            )
            .bind(id)
            .bind(&session.tenant_id)
            .bind(&session.user_id)
            .fetch_optional(&self.pool)
            .await?;
            let Some(row) = row else {
                // 设备已结束或过期清理，浏览器据此停止轮询。
                return Ok(TerminalEvents {
                    state: "closed".into(),
                    frames: Vec::new(),
                });
            };
            let state: String = row.try_get("state")?;
            let frames = sqlx::query(
                "SELECT cursor,kind,data FROM worker_terminal_frames WHERE session_id=$1 AND direction='output' AND cursor>$2 ORDER BY cursor ASC LIMIT 256",
            )
            .bind(id)
            .bind(after as i64)
            .fetch_all(&self.pool)
            .await?;
            if !frames.is_empty() {
                let cursor: i64 = frames.last().unwrap().try_get("cursor")?;
                sqlx::query("UPDATE worker_terminal_sessions SET browser_cursor=$2,updated_at=now() WHERE id=$1")
                    .bind(id)
                    .bind(cursor)
                    .execute(&self.pool)
                    .await?;
                return Ok(TerminalEvents {
                    state,
                    frames: frames
                        .into_iter()
                        .map(terminal_frame)
                        .collect::<Result<Vec<_>>>()?,
                });
            }
            // 关闭中或已关闭立即返回；等待设备接管时继续长轮询。
            if matches!(state.as_str(), "closing" | "closed") {
                return Ok(TerminalEvents {
                    state,
                    frames: Vec::new(),
                });
            }
            if std::time::Instant::now() >= deadline {
                return Ok(TerminalEvents {
                    state,
                    frames: Vec::new(),
                });
            }
            // 长轮询期间刷新活跃时间，避免用户未输入时静默会话被清理。
            sqlx::query("UPDATE worker_terminal_sessions SET updated_at=now() WHERE id=$1")
                .bind(id)
                .execute(&self.pool)
                .await?;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
    async fn terminal_input(
        &self,
        session: &SessionContext,
        id: &str,
        request: TerminalInput,
    ) -> Result<TerminalSession> {
        self.terminal_input_frame(session, id, "data", &request.data)
            .await
    }
    async fn terminal_resize(
        &self,
        session: &SessionContext,
        id: &str,
        request: TerminalResize,
    ) -> Result<TerminalSession> {
        validate_terminal_size(request.cols, request.rows)?;
        self.terminal_input_frame(session, id, "resize", &serde_json::to_string(&request)?)
            .await
    }
    async fn terminal_close(&self, session: &SessionContext, id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT state FROM worker_terminal_sessions WHERE id=$1 AND tenant_id=$2 AND user_id=$3 FOR UPDATE",
        )
        .bind(id)
        .bind(&session.tenant_id)
        .bind(&session.user_id)
        .fetch_optional(&mut *tx)
        .await?
        .context("终端会话不存在")?;
        let state: String = row.try_get("state")?;
        if state == "waiting" {
            sqlx::query("DELETE FROM worker_terminal_sessions WHERE id=$1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        } else if state != "closed" {
            sqlx::query(
                "UPDATE worker_terminal_sessions SET state='closing',updated_at=now() WHERE id=$1",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    async fn terminal_claim(
        &self,
        device: &DeviceIdentity,
        wait_seconds: u8,
    ) -> Result<Option<TerminalSession>> {
        self.expire_terminals().await?;
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(u64::from(wait_seconds));
        loop {
            let mut tx = self.pool.begin().await?;
            let active: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM worker_devices WHERE id=$1 AND state='active')",
            )
            .bind(&device.id)
            .fetch_one(&mut *tx)
            .await?;
            ensure!(active, "设备已撤销");
            let row = sqlx::query(
                "SELECT *, (extract(epoch FROM created_at)*1000)::bigint AS created_at_ms FROM worker_terminal_sessions WHERE worker_id=$1 AND state='waiting' ORDER BY created_at FOR UPDATE SKIP LOCKED LIMIT 1",
            )
            .bind(&device.id)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(row) = row {
                let id: String = row.try_get("id")?;
                sqlx::query("UPDATE worker_terminal_sessions SET state='active',updated_at=now() WHERE id=$1")
                    .bind(&id)
                    .execute(&mut *tx)
                    .await?;
                let mut value = terminal(row)?;
                value.state = "active".into();
                tx.commit().await?;
                return Ok(Some(value));
            }
            tx.commit().await?;
            if std::time::Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
    async fn terminal_read(
        &self,
        device: &DeviceIdentity,
        id: &str,
        after: u64,
        wait_seconds: u8,
    ) -> Result<TerminalEvents> {
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(u64::from(wait_seconds));
        loop {
            let row = sqlx::query(
                "SELECT state FROM worker_terminal_sessions WHERE id=$1 AND worker_id=$2",
            )
            .bind(id)
            .bind(&device.id)
            .fetch_optional(&self.pool)
            .await?;
            let Some(row) = row else {
                // 浏览器已关闭或会话过期，设备据此结束本地 PTY。
                return Ok(TerminalEvents {
                    state: "closed".into(),
                    frames: Vec::new(),
                });
            };
            let state: String = row.try_get("state")?;
            let frames = sqlx::query(
                "SELECT cursor,kind,data FROM worker_terminal_frames WHERE session_id=$1 AND direction='input' AND cursor>$2 ORDER BY cursor ASC LIMIT 256",
            )
            .bind(id)
            .bind(after as i64)
            .fetch_all(&self.pool)
            .await?;
            if !frames.is_empty() {
                let cursor: i64 = frames.last().unwrap().try_get("cursor")?;
                sqlx::query("UPDATE worker_terminal_sessions SET device_cursor=$2,updated_at=now() WHERE id=$1")
                    .bind(id)
                    .bind(cursor)
                    .execute(&self.pool)
                    .await?;
                return Ok(TerminalEvents {
                    state,
                    frames: frames
                        .into_iter()
                        .map(terminal_frame)
                        .collect::<Result<Vec<_>>>()?,
                });
            }
            if matches!(state.as_str(), "closing" | "closed") {
                return Ok(TerminalEvents {
                    state,
                    frames: Vec::new(),
                });
            }
            if std::time::Instant::now() >= deadline {
                return Ok(TerminalEvents {
                    state,
                    frames: Vec::new(),
                });
            }
            // 设备长时间无输入时保持会话活跃，避免被浏览器长轮询间隙判为过期。
            sqlx::query("UPDATE worker_terminal_sessions SET updated_at=now() WHERE id=$1")
                .bind(id)
                .execute(&self.pool)
                .await?;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
    async fn terminal_write(
        &self,
        device: &DeviceIdentity,
        id: &str,
        request: TerminalInput,
    ) -> Result<()> {
        validate_terminal_data(&request.data, 64 * 1024)?;
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT state FROM worker_terminal_sessions WHERE id=$1 AND worker_id=$2 FOR UPDATE",
        )
        .bind(id)
        .bind(&device.id)
        .fetch_optional(&mut *tx)
        .await?
        .context("终端会话不存在")?;
        let state: String = row.try_get("state")?;
        ensure!(state != "closed" && state != "closing", "终端会话正在关闭");
        let cursor: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(cursor),0)+1 FROM worker_terminal_frames WHERE session_id=$1 AND direction='output'",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO worker_terminal_frames(session_id,direction,cursor,kind,data) VALUES($1,'output',$2,'data',$3)")
            .bind(id)
            .bind(cursor)
            .bind(&request.data)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE worker_terminal_sessions SET updated_at=now() WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    async fn terminal_finish(
        &self,
        device: &DeviceIdentity,
        id: &str,
        request: TerminalFinish,
    ) -> Result<()> {
        ensure!(request.reason.len() <= 120, "终端结束原因过长");
        let n = sqlx::query(
            "DELETE FROM worker_terminal_sessions WHERE id=$1 AND worker_id=$2 AND state IN ('active','closing')",
        )
        .bind(id)
        .bind(&device.id)
        .execute(&self.pool)
        .await?
        .rows_affected();
        ensure!(n == 1, "终端会话不存在或已结束");
        Ok(())
    }
    async fn terminal_access(&self, device: &DeviceIdentity, enabled: bool) -> Result<()> {
        self.local_capability(device, "terminal.open", enabled)
            .await?;
        if !enabled {
            sqlx::query("DELETE FROM worker_terminal_sessions WHERE worker_id=$1")
                .bind(&device.id)
                .execute(&self.pool)
                .await?;
        }
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
    async fn terminal_input_frame(
        &self,
        session: &SessionContext,
        id: &str,
        kind: &str,
        data: &str,
    ) -> Result<TerminalSession> {
        validate_terminal_data(data, 64 * 1024)?;
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT *, (extract(epoch FROM created_at)*1000)::bigint AS created_at_ms FROM worker_terminal_sessions WHERE id=$1 AND tenant_id=$2 AND user_id=$3 FOR UPDATE",
        )
        .bind(id)
        .bind(&session.tenant_id)
        .bind(&session.user_id)
        .fetch_optional(&mut *tx)
        .await?
        .context("终端会话不存在")?;
        let state: String = row.try_get("state")?;
        ensure!(state != "closed" && state != "closing", "终端会话正在关闭");
        let cursor: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(cursor),0)+1 FROM worker_terminal_frames WHERE session_id=$1 AND direction='input'",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO worker_terminal_frames(session_id,direction,cursor,kind,data) VALUES($1,'input',$2,$3,$4)")
            .bind(id)
            .bind(cursor)
            .bind(kind)
            .bind(data)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE worker_terminal_sessions SET updated_at=now() WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let value = terminal(row)?;
        tx.commit().await?;
        Ok(value)
    }
    async fn expire_terminals(&self) -> Result<()> {
        sqlx::query(
            "DELETE FROM worker_terminal_sessions WHERE (state='waiting' AND updated_at<now()-interval '10 minutes') OR (state IN ('active','closing') AND updated_at<now()-interval '2 hours')",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
    async fn expire(&self) -> Result<()> {
        // 未知是否完成的副作用任务不自动重派，由用户检查结果后重试。
        sqlx::query("UPDATE worker_tasks SET state='interrupted',error='设备租约过期，执行结果待确认',lease=NULL,lease_until=NULL WHERE state='running' AND lease_until<now()")
            .execute(&self.pool).await?;
        Ok(())
    }
}

impl WorkerServiceImpl {
    async fn local_capability(
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
