use super::model::{Task, Worker};
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use sqlx::Row;

pub(super) fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
pub(super) fn secret() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| anyhow::anyhow!("生成设备凭据失败"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub(super) fn validate_capability(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 80
            && value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b)),
        "能力名称无效"
    );
    Ok(())
}
/// 宿主只验证批次协议；命令、路径和并行执行由设备本机授权与处理。
pub(super) fn validate_workspace_input(input: &serde_json::Value) -> Result<()> {
    match input.get("action").and_then(serde_json::Value::as_str) {
        Some("describe") => Ok(()),
        Some("run") => {
            ensure!(
                input
                    .get("jobs")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|jobs| (1..=8).contains(&jobs.len())),
                "工作区批次需要 1 至 8 个任务"
            );
            Ok(())
        }
        _ => anyhow::bail!("工作区操作无效"),
    }
}
pub(super) fn worker(row: sqlx::postgres::PgRow) -> Result<Worker> {
    Ok(Worker {
        id: row.try_get("id")?,
        label: row.try_get("label")?,
        platform: row.try_get("platform")?,
        capabilities: serde_json::from_value(row.try_get("capabilities")?)?,
        status: row.try_get("status")?,
        last_seen: row.try_get("last_seen_ms")?,
    })
}
pub(super) fn task(row: sqlx::postgres::PgRow) -> Result<Task> {
    Ok(Task {
        id: row.try_get("id")?,
        worker_id: row.try_get("worker_id")?,
        capability: row.try_get("capability")?,
        input: row.try_get("input")?,
        state: row.try_get("state")?,
        result: row.try_get("result")?,
        error: row.try_get("error")?,
        lease: row.try_get("lease")?,
        created_at: row.try_get("created_at_ms")?,
    })
}
