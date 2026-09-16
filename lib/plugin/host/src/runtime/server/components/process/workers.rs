use super::{broker, model::Gateway};
use crate::{generated::worker::model::SubmitTask, identity::SessionContext};
use anyhow::{Context, Result, ensure};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

const CAPABILITY: &str = "desktop.open-app";

/// 进程身份与用户范围由宿主验证，模型参数不能指定其他账号。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Request {
    tenant_id: String,
    user_id: String,
    operation: String,
    worker_id: Option<String>,
    application: Option<String>,
    request_id: Option<String>,
    task_id: Option<String>,
}

pub(super) async fn invoke(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> Response {
    match execute(&gateway, &headers, request).await {
        Ok(value) => Json(value).into_response(),
        Err(_) => (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"设备不可用或调用未授权"})),
        )
            .into_response(),
    }
}

async fn execute(gateway: &Gateway, headers: &HeaderMap, request: Request) -> Result<Value> {
    let _permit = gateway.quota.try_acquire().context("设备调用并发已满")?;
    let components = broker::active(gateway, headers).await?;
    ensure!(
        request.tenant_id == gateway.start.tenant
            && gateway
                .worker_capabilities
                .iter()
                .any(|capability| capability == CAPABILITY),
        "插件未获设备授权"
    );
    let actor: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM component_process_actors WHERE source_id=$1 AND tenant_id=$2 AND user_id=$3)")
        .bind(gateway.start.source).bind(&request.tenant_id).bind(&request.user_id).fetch_one(&components.pool).await?;
    ensure!(
        actor
            && components
                .identity
                .member_active(&request.tenant_id, &request.user_id)
                .await?,
        "设备用户已撤权"
    );
    let session = SessionContext {
        session_id: format!("service:{}", gateway.start.source),
        user_id: request.user_id,
        tenant_id: request.tenant_id,
        account: String::new(),
        display_name: String::new(),
        tenant_label: String::new(),
        permissions: Vec::new(),
    };
    match request.operation.as_str() {
        "list" => {
            let devices = components.workers.list(&session).await?;
            let devices: Vec<_> = devices
                .into_iter()
                .filter(|device| {
                    device.status != "revoked"
                        && device
                            .capabilities
                            .iter()
                            .any(|capability| capability == CAPABILITY)
                })
                .collect();
            Ok(serde_json::to_value(devices)?)
        }
        "openApp" => {
            let application = request.application.context("应用名称缺失")?;
            validate_application(&application)?;
            let task = components
                .workers
                .enqueue(
                    &session,
                    SubmitTask {
                        id: request.request_id.context("请求 ID 缺失")?,
                        worker_id: request.worker_id.context("设备 ID 缺失")?,
                        capability: CAPABILITY.into(),
                        input: json!({"application":application}),
                    },
                )
                .await?;
            Ok(serde_json::to_value(task)?)
        }
        "task" => {
            let task = components
                .workers
                .task(&session, &request.task_id.context("任务 ID 缺失")?)
                .await?;
            ensure!(task.capability == CAPABILITY, "任务能力未授权");
            Ok(serde_json::to_value(task)?)
        }
        _ => anyhow::bail!("不支持的设备操作"),
    }
}

fn validate_application(application: &str) -> Result<()> {
    ensure!(
        !application.trim().is_empty()
            && application.len() <= 128
            && !application.starts_with('-')
            && application
                .chars()
                .all(|ch| ch.is_alphanumeric() || " ._+-".contains(ch)),
        "应用名称无效"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn applications_do_not_accept_shell_or_paths() {
        for value in ["Postman", "Google Chrome", "com.postmanlabs.mac", "备忘录"] {
            assert!(validate_application(value).is_ok());
        }
        for value in [
            "",
            "--args",
            "/Applications/Postman.app",
            "Postman; curl evil",
            "$(id)",
            "Postman\nopen Terminal",
        ] {
            assert!(validate_application(value).is_err());
        }
    }
}
