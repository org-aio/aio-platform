use super::*;

impl WorkerServiceImpl {
    pub(super) async fn terminal_input_frame(
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
    pub(super) async fn expire_terminals(&self) -> Result<()> {
        sqlx::query(
            "DELETE FROM worker_terminal_sessions WHERE (state='waiting' AND updated_at<now()-interval '10 minutes') OR (state IN ('active','closing') AND updated_at<now()-interval '2 hours')",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
    pub(super) async fn expire(&self) -> Result<()> {
        // 未知是否完成的副作用任务不自动重派，由用户检查结果后重试。
        sqlx::query("UPDATE worker_tasks SET state='interrupted',error='设备租约过期，执行结果待确认',lease=NULL,lease_until=NULL WHERE state='running' AND lease_until<now()")
            .execute(&self.pool).await?;
        Ok(())
    }
}
