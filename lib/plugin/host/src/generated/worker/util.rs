use super::model::{Task, TerminalFrame, TerminalSession, Worker};
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
        Some("add") => {
            ensure!(
                input.as_object().is_some_and(|value| value.len() == 1),
                "添加项目不接受路径或名称"
            );
            Ok(())
        }
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
/// ADB 页面只提交有限动作；目标序列号和参数在设备端仍会再次校验。
pub(super) fn validate_adb_input(input: &serde_json::Value) -> Result<()> {
    let object = input
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("ADB 输入必须为对象"))?;
    ensure!(
        object
            .keys()
            .all(|key| ["action", "target", "arguments"].contains(&key.as_str())),
        "ADB 输入字段无效"
    );
    let action = input["action"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("ADB 动作缺失"))?;
    ensure!(
        [
            "list_devices",
            "pair",
            "connect",
            "disconnect",
            "device_info",
            "screenshot",
            "shell",
            "input_key",
            "input_text",
            "tap",
            "swipe",
            "reboot",
            "getprop",
            "logcat",
            "list_packages",
            "launch_app",
            "force_stop",
            "install_apk",
            "push",
            "pull"
        ]
        .contains(&action),
        "ADB 动作未开放"
    );
    ensure!(input["arguments"].is_object(), "ADB 参数必须为对象");
    if let Some(target) = input.get("target") {
        let target = target
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("ADB 目标必须是字符串"))?;
        ensure!(
            !target.is_empty()
                && target.len() <= 128
                && !target.starts_with('-')
                && target
                    .bytes()
                    .all(|byte| { byte.is_ascii_alphanumeric() || b"._:-".contains(&byte) }),
            "ADB 目标无效"
        );
    }
    match action {
        "pair" => {
            ensure!(
                input["arguments"]["host"]
                    .as_str()
                    .is_some_and(|value| !value.is_empty() && value.len() <= 253),
                "无线配对地址无效"
            );
            ensure!(
                input["arguments"]["port"]
                    .as_u64()
                    .is_some_and(|value| value <= 65_535),
                "无线配对端口无效"
            );
            ensure!(
                input["arguments"]["code"]
                    .as_str()
                    .is_some_and(|value| (6..=12).contains(&value.len())
                        && value.bytes().all(|byte| byte.is_ascii_digit())),
                "无线配对码无效"
            );
        }
        "connect" | "disconnect" => ensure!(
            input["arguments"]["host"]
                .as_str()
                .is_some_and(|value| !value.is_empty() && value.len() <= 253),
            "ADB 主机地址无效"
        ),
        "list_devices" => {}
        _ => ensure!(
            input
                .get("target")
                .and_then(serde_json::Value::as_str)
                .is_some(),
            "ADB 目标设备缺失"
        ),
    }
    ensure!(
        serde_json::to_vec(&input["arguments"])?.len() <= 30_000,
        "ADB 参数过大"
    );
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
pub(super) fn terminal(row: sqlx::postgres::PgRow) -> Result<TerminalSession> {
    Ok(TerminalSession {
        id: row.try_get("id")?,
        worker_id: row.try_get("worker_id")?,
        state: row.try_get("state")?,
        cols: row.try_get::<i32, _>("cols")?.try_into()?,
        rows: row.try_get::<i32, _>("rows")?.try_into()?,
        created_at: row.try_get("created_at_ms")?,
    })
}
pub(super) fn terminal_frame(row: sqlx::postgres::PgRow) -> Result<TerminalFrame> {
    Ok(TerminalFrame {
        cursor: row.try_get::<i64, _>("cursor")?.try_into()?,
        kind: row.try_get("kind")?,
        data: row.try_get("data")?,
    })
}
/// 终端输入和尺寸只接受有限协议，正文本身在设备端按 UTF-8 字节处理。
pub(super) fn validate_terminal_data(value: &str, limit: usize) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= limit,
        "终端数据长度无效"
    );
    Ok(())
}
pub(super) fn validate_terminal_size(cols: u16, rows: u16) -> Result<()> {
    ensure!(
        (20..=500).contains(&cols) && (5..=300).contains(&rows),
        "终端尺寸无效"
    );
    Ok(())
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

#[cfg(test)]
mod adb_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn actions_and_targets_are_bounded() -> Result<()> {
        validate_adb_input(&json!({"action":"list_devices","arguments":{}}))?;
        validate_adb_input(&json!({
            "action":"input_key",
            "target":"192.168.1.8:5555",
            "arguments":{"key":"KEYCODE_DPAD_DOWN"}
        }))?;
        validate_adb_input(&json!({
            "action":"pair",
            "arguments":{"host":"192.168.1.8","port":37021,"code":"123456"}
        }))?;
        assert!(
            validate_adb_input(&json!({"action":"shell","arguments":{"command":"id"}})).is_err()
        );
        assert!(
            validate_adb_input(&json!({
                "action":"input_key",
                "target":"-s",
                "arguments":{"key":"KEYCODE_HOME"}
            }))
            .is_err()
        );
        assert!(validate_adb_input(&json!({"action":"raw","arguments":{}})).is_err());
        Ok(())
    }
}

#[cfg(test)]
mod terminal_tests {
    use super::*;
    #[test]
    fn data_length_bounds_are_enforced() -> Result<()> {
        assert!(validate_terminal_data("", 16).is_err());
        validate_terminal_data("ls\r", 16)?;
        assert!(validate_terminal_data(&"a".repeat(17), 16).is_err());
        validate_terminal_data(&"a".repeat(16), 16)?;
        Ok(())
    }
    #[test]
    fn size_bounds_are_enforced() -> Result<()> {
        validate_terminal_size(20, 5)?;
        validate_terminal_size(500, 300)?;
        assert!(validate_terminal_size(19, 24).is_err());
        assert!(validate_terminal_size(80, 4).is_err());
        assert!(validate_terminal_size(501, 24).is_err());
        assert!(validate_terminal_size(80, 301).is_err());
        Ok(())
    }
}
