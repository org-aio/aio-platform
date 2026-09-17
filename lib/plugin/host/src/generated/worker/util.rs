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
/// 桌面仅接受固定 OCU 工具；本机仍需独立开启授权并验证观察凭据。
pub(super) fn validate_desktop_input(input: &serde_json::Value) -> Result<()> {
    let object = input
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("桌面输入必须为对象"))?;
    ensure!(
        object
            .keys()
            .all(|key| ["session", "action", "arguments", "observation"].contains(&key.as_str())),
        "桌面输入字段无效"
    );
    uuid::Uuid::parse_str(
        input["session"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("桌面会话缺失"))?,
    )?;
    let action = input["action"].as_str().unwrap_or_default();
    ensure!(
        [
            "list_apps",
            "get_app_state",
            "activate_app",
            "click",
            "type_text",
            "press_key",
            "scroll",
            "drag",
            "set_value",
            "perform_secondary_action",
            "create_spreadsheet",
            "release"
        ]
        .contains(&action),
        "桌面动作未开放"
    );
    ensure!(input["arguments"].is_object(), "桌面参数必须为对象");
    if !["list_apps", "release"].contains(&action) {
        ensure!(
            input["arguments"]["app"]
                .as_str()
                .is_some_and(|app| !app.trim().is_empty() && app.len() <= 256),
            "应用名称无效"
        );
    }
    if !["list_apps", "get_app_state", "activate_app", "release"].contains(&action) {
        uuid::Uuid::parse_str(
            input["observation"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("需要最新界面观察凭据"))?,
        )?;
    }
    Ok(())
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

#[cfg(test)]
mod desktop_tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;
    #[test]
    fn actions_require_fresh_observation() -> Result<()> {
        let input = json!({"session":Uuid::new_v4(),"action":"click","arguments":{"app":"WPS","element_index":"1"}});
        assert!(validate_desktop_input(&input).is_err());
        let mut observed = input;
        observed["observation"] = json!(Uuid::new_v4());
        validate_desktop_input(&observed)?;
        observed["action"] = json!("create_spreadsheet");
        observed["arguments"] =
            json!({"app":"WPS","filename":"人员.xlsx","rows":[["姓名","年龄"],["小明",18]]});
        validate_desktop_input(&observed)?;
        observed["action"] = json!("shell");
        assert!(validate_desktop_input(&observed).is_err());
        Ok(())
    }
}
