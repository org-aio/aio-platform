//! 明确声明的匿名资源入口；与有身份的组件调用分开，不能转授用户能力。
use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use axum::{
    body::Body,
    extract::{Path, RawQuery, State},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
};
use az_plugin_contract::{public::PUBLIC_TIMEOUT_SECONDS, transport::Request};
use tokio::{sync::OwnedSemaphorePermit, time::Instant};
use uuid::Uuid;

use super::super::RuntimeState;
use super::Components;

impl Components {
    pub(super) async fn public_generation(
        &self,
        source: Uuid,
        tenant: &str,
        digest: &str,
    ) -> Result<String> {
        ensure!(
            self.is_active(source, tenant, digest).await,
            "公开资源实例未激活"
        );
        if tenant == "development"
            && let Some(instance) = self.development.read().await.get(&source)
        {
            ensure!(instance.bundle.digest() == digest, "公开资源版本已变化");
            return Ok(instance.generation.clone());
        }
        let entry = self
            .current_installation(tenant, source)
            .await?
            .context("插件未安装")?;
        let installed = entry.installation.context("插件未安装")?;
        ensure!(
            installed.enabled && installed.digest == digest,
            "公开资源版本已撤销"
        );
        Ok(installed.generation.to_string())
    }
}

pub(super) async fn read(
    State(state): State<RuntimeState>,
    Path((source, tenant, path)): Path<(Uuid, String, String)>,
    RawQuery(query): RawQuery,
    method: Method,
) -> Response {
    let Ok(components) = state.components() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let Ok(permit) = components.public_slots.clone().try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    if tenant.is_empty()
        || tenant.len() > 128
        || tenant.chars().any(char::is_control)
        || query
            .as_ref()
            .is_some_and(|q| q.len() > 8192 || q.chars().any(char::is_control))
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Ok(tasks) = state.plugin_tasks::<super::super::plugins::ComponentsPlugin>() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let deadline = Instant::now() + Duration::from_secs(PUBLIC_TIMEOUT_SECONDS);
    let request = Request {
        method: method.to_string(),
        path: format!("/{path}"),
        query,
        headers: vec![],
        body: vec![],
    };
    match tokio::time::timeout_at(
        deadline,
        prepare(components, source, tenant, request, permit, deadline, tasks),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(_)) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::GATEWAY_TIMEOUT.into_response(),
    }
}

async fn prepare(
    components: Arc<Components>,
    source: Uuid,
    tenant: String,
    request: Request,
    permit: OwnedSemaphorePermit,
    deadline: Instant,
    tasks: Arc<super::super::plugins::PluginTasks>,
) -> Result<Response> {
    let bundle = components.bundle(source, &tenant).await?;
    let digest = bundle.digest().to_owned();
    let route = bundle
        .manifest()
        .plugin
        .public_routes
        .iter()
        .find(|route| route.allows(&request.method, &request.path))
        .context("资源未公开")?
        .clone();
    let generation = components
        .public_generation(source, &tenant, &digest)
        .await?;
    let executor = components.active_executor(source, &tenant, &digest).await?;
    let head = request.method == "HEAD";
    let response = executor
        .handle_public(source, &tenant, &digest, request)
        .await?;
    let binding = super::streaming::Access::Public {
        components,
        source,
        tenant,
        digest,
        generation,
    };
    binding.check().await?;
    if matches!(response.status, 404 | 410) {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    ensure!(
        response.status == 200 && response.headers.len() <= 16,
        "公开资源响应无效"
    );
    let header = |name: &str| -> Result<Option<&str>> {
        let mut values = response
            .headers
            .iter()
            .filter(|h| h.name.eq_ignore_ascii_case(name));
        let value = values.next().map(|h| h.value.as_str());
        ensure!(values.next().is_none(), "公开资源响应头重复");
        Ok(value)
    };
    let content_type = header("content-type")?.context("缺少公开资源类型")?;
    ensure!(
        content_type.len() <= 256
            && route
                .content_types
                .iter()
                .any(|mime| content_type.split(';').next().map(str::trim) == Some(mime.as_str())),
        "公开资源类型未声明"
    );
    let expected: Option<u64> = header("content-length")?.map(str::parse).transpose()?;
    ensure!(
        expected.is_none_or(|length| length <= route.max_bytes),
        "公开资源超过配额"
    );
    let mut builder = Response::builder()
        .header("content-type", content_type)
        .header("cache-control", "no-store")
        .header("x-content-type-options", "nosniff")
        // 内联页面仍使用无脚本沙箱：只允许缩进样式和包内图片，不授予同源或网络能力。
        .header(
            "content-security-policy",
            if route.inline {
                "sandbox; default-src 'none'; img-src data:; style-src 'unsafe-inline'; frame-ancestors 'none'"
            } else {
                "sandbox; default-src 'none'; img-src data:; frame-ancestors 'none'"
            },
        )
        .header("referrer-policy", "no-referrer")
        .header("cross-origin-resource-policy", "cross-origin")
        .header(
            "content-disposition",
            if route.inline || content_type.starts_with("image/") {
                "inline"
            } else {
                "attachment"
            },
        );
    if let Some(length) = expected {
        builder = builder.header("content-length", length);
    }
    if head {
        return Ok(builder.body(Body::empty())?);
    }
    let transfer = super::streaming::Transfer {
        access: binding,
        tasks,
        limit: route.max_bytes,
        body: response.body,
        bytes: 0,
        expected,
        deadline,
        next_check: Instant::now(),
        _permit: permit,
    };
    Ok(builder.body(transfer.body()?)?)
}
