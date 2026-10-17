//! 受信任组件复用正式管理路由。内部会话绑定到当前 Future，不传 cookie、不接受身份头。

use std::sync::{Arc, Weak};

use anyhow::{Context, Result, ensure};
use axum::body::{Body, to_bytes};
use az_plugin_contract::InvocationScope;
use az_plugin_contract::transport::{Header, Request, Response};
use tower::ServiceExt;

use super::{RuntimeState, components::services::Services, plugins::ComponentManagementPlugin};
use crate::identity::SessionContext;

tokio::task_local! {
    static DELEGATED_SESSION: Delegation;
}

struct Delegation {
    scope: InvocationScope,
    services: Weak<Services>,
}

pub(super) fn session() -> Result<Option<SessionContext>> {
    DELEGATED_SESSION
        .try_with(|delegation| {
            let services = delegation.services.upgrade().context("组件宿主已关闭")?;
            let (session, _) = services.management_session(&delegation.scope)?;
            Ok(Some(session))
        })
        .unwrap_or(Ok(None))
}

pub(super) async fn invoke(
    state: RuntimeState,
    scope: InvocationScope,
    services: Arc<Services>,
    executing: bool,
    request: Request,
) -> Result<Response> {
    validate(&request)?;
    prevent_execution_mutation(executing, &request)?;
    let tasks = state.plugin_tasks::<ComponentManagementPlugin>()?;
    let _permit = tasks.enter().await?;
    let uri = match &request.query {
        Some(query) => format!("{}?{query}", request.path),
        None => request.path.clone(),
    };
    let mut builder = axum::http::Request::builder()
        .method(request.method.as_str())
        .uri(uri);
    for header in request.headers {
        builder = builder.header(header.name, header.value);
    }
    let input = builder.body(Body::from(request.body))?;
    let router = super::routes::assemble(state)?;
    // 路由的现有管理、发布、租户和设备检查全部保留；子任务不能继承此交互凭证。
    DELEGATED_SESSION
        .scope(
            Delegation {
                scope,
                services: Arc::downgrade(&services),
            },
            async move {
                let output = router.oneshot(input).await?;
                let status = output.status().as_u16();
                let headers = output
                    .headers()
                    .iter()
                    .filter(|(name, _)| {
                        matches!(name.as_str(), "content-type" | "content-disposition")
                    })
                    .map(|(name, value)| {
                        Ok(Header {
                            name: name.to_string(),
                            value: value.to_str()?.into(),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let body = to_bytes(output.into_body(), 32 * 1024 * 1024)
                    .await?
                    .to_vec();
                Ok(Response {
                    status,
                    headers,
                    body,
                })
            },
        )
        .await
}

fn validate(request: &Request) -> Result<()> {
    ensure!(
        request.body.len() <= 8 * 1024 * 1024 && request.headers.len() <= 8,
        "管理请求超过配额"
    );
    ensure!(
        request.path.len() <= 2048
            && request
                .path
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/_-.~".contains(&b))
            && !request.path.split('/').any(|s| s == "." || s == ".."),
        "管理路径无效"
    );
    ensure!(
        request.query.as_ref().is_none_or(|q| q.len() <= 8192
            && !q.chars().any(char::is_control)
            && !q.contains('#')),
        "管理查询无效"
    );
    for header in &request.headers {
        ensure!(
            ["content-type", "accept"].contains(&header.name.to_ascii_lowercase().as_str())
                && header.value.len() <= 256
                && !header.value.chars().any(char::is_control),
            "管理调用不能注入身份或宿主请求头"
        );
    }
    let parts = request.path.split('/').collect::<Vec<_>>();
    let Some(["", "api", "runtime", tail @ ..]) = parts.get(..) else {
        anyhow::bail!("管理路径不属于宿主管理契约");
    };
    let permitted = match (request.method.as_str(), tail) {
        ("GET", ["catalog" | "marketplace" | "releases"]) => true,
        ("GET", ["packages" | "publish-jobs", _]) => true,
        ("GET", ["plugins", _, "events"]) => true,
        ("GET", ["marketplace", _, "details"]) => true,
        ("GET", ["marketplace", _, "images", rest @ ..]) => !rest.is_empty(),
        ("GET", ["tools", "access"]) => true,
        ("GET", ["tools", _, "details" | "devices"]) => true,
        ("GET", ["workers", "tasks"]) => true,
        ("POST", ["plugins", "install"]) => true,
        (
            "POST",
            [
                "plugins",
                source,
                "enable" | "disable" | "rollback" | "uninstall" | "hide-menu" | "show-menu",
            ],
        ) => uuid::Uuid::parse_str(source).is_ok(),
        ("POST", ["delivery", "jobs", _, "retry"]) => true,
        ("POST", ["tools", "register"]) => true,
        ("POST", ["tools", _, "install"]) => true,
        ("PATCH", ["tools", _, "details"]) => true,
        ("DELETE", ["marketplace"]) | ("DELETE", ["tools", _]) => true,
        _ => false,
    };
    ensure!(permitted, "请求不属于已开放的宿主管理契约");
    Ok(())
}

fn prevent_execution_mutation(executing: bool, request: &Request) -> Result<()> {
    if !executing || request.method != "POST" {
        return Ok(());
    }
    // 不仅自身变更会等待自己，其他升级也可能持有全局变更锁并等待本次执行结束。
    ensure!(
        !request.path.starts_with("/api/runtime/plugins/"),
        "执行中的插件不能同步变更插件生命周期；请通过前端管理入口执行"
    );
    Ok(())
}

#[cfg(test)]
#[path = "management_dispatch_tests.rs"]
mod tests;
