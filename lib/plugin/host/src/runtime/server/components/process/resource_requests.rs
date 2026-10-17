use anyhow::{Context, Result, ensure};
use futures_util::StreamExt;
use uuid::Uuid;

use super::super::execution::{Request, StreamingResponse};
use super::Processes;

impl Processes {
    pub async fn handle_stream(
        &self,
        source: Uuid,
        tenant: &str,
        digest: &str,
        request: Request,
        context: az_plugin_contract::RequestContext,
    ) -> Result<StreamingResponse> {
        ensure!(
            matches!(request.method.as_str(), "GET" | "HEAD") && request.body.is_empty(),
            "流式入口仅允许读取"
        );
        let instance = self
            .instances
            .lock()
            .await
            .get(&(source, tenant.into()))
            .cloned()
            .context("process 未激活")?;
        ensure!(
            instance.bundle.digest() == digest && context.tenant_id.as_deref() == Some(tenant),
            "process 活动版本或租户已变化"
        );
        let user = context.user_id.context("process 调用缺少用户")?;
        self.components
            .upgrade()
            .context("宿主已停止")?
            .catalog
            .record_process_actor(source, tenant, &user)
            .await?;
        let mut url = reqwest::Url::parse("http://localhost")?;
        url.set_path(&request.path);
        url.set_query(request.query.as_deref());
        let mut call = instance
            .client
            .request(reqwest::Method::from_bytes(request.method.as_bytes())?, url)
            .timeout(std::time::Duration::from_secs(
                az_plugin_contract::public::PUBLIC_TIMEOUT_SECONDS,
            ))
            .header("x-aio-token", &instance.token)
            .header("x-aio-tenant-id", tenant)
            .header("x-aio-user-id", user)
            .header("x-aio-context", context.request_id);
        for header in request.headers {
            if ["accept", "content-type"].contains(&header.name.to_ascii_lowercase().as_str()) {
                call = call.header(header.name, header.value);
            }
        }
        stream_response(call.send().await.context("process 流式读取失败")?)
    }

    pub async fn handle_public(
        &self,
        source: Uuid,
        tenant: &str,
        digest: &str,
        request: Request,
    ) -> Result<StreamingResponse> {
        let instance = self
            .instances
            .lock()
            .await
            .get(&(source, tenant.into()))
            .cloned()
            .context("process 未激活")?;
        ensure!(instance.bundle.digest() == digest, "process 活动版本已变化");
        ensure!(
            request.body.is_empty()
                && instance
                    .bundle
                    .manifest()
                    .plugin
                    .public_routes
                    .iter()
                    .any(|route| route.allows(&request.method, &request.path)),
            "未声明公开资源"
        );
        let mut url = reqwest::Url::parse("http://localhost")?;
        url.set_path(&request.path);
        url.set_query(request.query.as_deref());
        // 公开调用不记录用户、不注册交互身份，也不转发客户端任何请求头。
        let response = instance
            .client
            .request(reqwest::Method::from_bytes(request.method.as_bytes())?, url)
            .header("x-aio-token", &instance.token)
            .header("x-aio-tenant-id", tenant)
            .header("x-aio-public", "1")
            .timeout(std::time::Duration::from_secs(
                az_plugin_contract::public::PUBLIC_TIMEOUT_SECONDS,
            ))
            .send()
            .await
            .context("process 公开资源调用失败")?;
        stream_response(response)
    }
}

fn stream_response(response: reqwest::Response) -> Result<StreamingResponse> {
    let status = response.status().as_u16();
    let mut headers = Vec::new();
    for name in ["content-type", "content-length"] {
        for value in response.headers().get_all(name) {
            headers.push(az_plugin_contract::transport::Header {
                name: name.into(),
                value: value.to_str()?.into(),
            });
        }
    }
    let body = response
        .bytes_stream()
        .map(|chunk| {
            chunk
                .map(|bytes| bytes.to_vec())
                .map_err(anyhow::Error::from)
        })
        .boxed();
    Ok(StreamingResponse {
        status,
        headers,
        body,
    })
}
