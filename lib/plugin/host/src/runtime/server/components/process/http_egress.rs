use super::{broker::active, model::Gateway};
use anyhow::{Context, Result, ensure};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use std::sync::Arc;

/// 只允许清单与宿主共同批准的完整 HTTPS URL，不接收任意方法、路径或转发头。
pub(super) async fn request(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match forward(&gateway, &headers, body).await {
        Ok(response) => response,
        Err(_) => (StatusCode::BAD_GATEWAY, "第三方服务不可用或未授权").into_response(),
    }
}

async fn forward(gateway: &Gateway, headers: &HeaderMap, body: Bytes) -> Result<Response> {
    let _permit = gateway.quota.clone().try_acquire_owned()?;
    active(gateway, headers).await?;
    let endpoint = headers
        .get("x-aio-endpoint")
        .and_then(|value| value.to_str().ok())
        .context("缺少出站地址")?;
    authorized(&gateway.http_endpoints, endpoint)?;
    let _: serde_json::Value = serde_json::from_slice(&body)?;
    let mut request = gateway
        .client
        .post(endpoint)
        .header("content-type", "application/json")
        .body(body)
        .timeout(std::time::Duration::from_secs(30));
    if let Some(authorization) = headers.get("authorization") {
        request = request.header("authorization", authorization);
    }
    let mut response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
        return Ok((status, "第三方服务拒绝请求").into_response());
    }
    ensure!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("application/json")),
        "第三方响应类型无效"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(bytes.len() + chunk.len() <= 512000, "第三方响应超过配额");
        bytes.extend_from_slice(&chunk);
    }
    Ok(([("content-type", "application/json")], bytes).into_response())
}

fn authorized(endpoints: &[String], requested: &str) -> Result<()> {
    ensure!(
        endpoints.iter().any(|endpoint| endpoint == requested),
        "第三方地址未授权"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn http_grant_is_an_exact_url_not_a_prefix_or_domain() {
        let endpoints = vec!["https://api.tavily.com/search".into()];
        assert!(super::authorized(&endpoints, "https://api.tavily.com/search").is_ok());
        for url in [
            "https://api.tavily.com/search/",
            "https://api.tavily.com/search?key=other",
            "https://api.tavily.com/search-extra",
            "https://api.tavily.com.evil.test/search",
            "http://api.tavily.com/search",
        ] {
            assert!(super::authorized(&endpoints, url).is_err());
        }
        assert!(super::authorized(&[], "https://api.tavily.com/search").is_err());
    }
}
