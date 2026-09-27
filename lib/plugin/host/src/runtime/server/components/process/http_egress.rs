use super::{broker::active, model::Gateway};
use anyhow::{Context, Result, ensure};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use std::sync::Arc;

/// 只允许清单与宿主共同批准的完整 HTTPS URL、受控 GET/POST 和白名单转发头。
pub(super) async fn request(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match forward(&gateway, &headers, body).await {
        Ok(response) => response,
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            format!("第三方服务不可用或未授权: {error}"),
        )
            .into_response(),
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
    let method = headers
        .get("x-aio-method")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("POST");
    let mut request = match method {
        "GET" => gateway.client.get(endpoint),
        "POST" => {
            let _: serde_json::Value = serde_json::from_slice(&body)?;
            gateway
                .client
                .post(endpoint)
                .header("content-type", "application/json")
                .body(body)
        }
        _ => anyhow::bail!("第三方 HTTP 方法未授权"),
    }
    .timeout(std::time::Duration::from_secs(30));
    for name in [
        "accept",
        "accept-language",
        "referer",
        "user-agent",
        "x-requested-with",
    ] {
        if let Some(value) = headers.get(name) {
            request = request.header(name, value);
        }
    }
    if let Some(authorization) = headers.get("authorization") {
        request = request.header("authorization", authorization);
    }
    let mut response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
        return Ok((status, "第三方服务拒绝请求").into_response());
    }
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .context("第三方响应缺少类型")?
        .to_owned();
    ensure!(
        content_type.starts_with("application/json") || content_type.starts_with("text/html"),
        "第三方响应类型无效"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(bytes.len() + chunk.len() <= 512000, "第三方响应超过配额");
        bytes.extend_from_slice(&chunk);
    }
    Ok(([("content-type", content_type)], bytes).into_response())
}

fn authorized(endpoints: &[String], requested: &str) -> Result<()> {
    let requested = reqwest::Url::parse(requested).context("第三方地址无效")?;
    let allowed = endpoints.iter().any(|endpoint| {
        reqwest::Url::parse(endpoint).is_ok_and(|endpoint| {
            endpoint.scheme() == requested.scheme()
                && endpoint.host_str() == requested.host_str()
                && endpoint.port_or_known_default() == requested.port_or_known_default()
                && endpoint.path() == requested.path()
        })
    });
    ensure!(allowed, "第三方地址未授权");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn http_grant_is_an_exact_url_not_a_prefix_or_domain() {
        let endpoints = vec!["https://api.tavily.com/search".into()];
        assert!(super::authorized(&endpoints, "https://api.tavily.com/search").is_ok());
        assert!(super::authorized(&endpoints, "https://api.tavily.com/search?q=rust").is_ok());
        for url in [
            "https://api.tavily.com/search/",
            "https://api.tavily.com/search-extra",
            "https://api.tavily.com.evil.test/search",
            "http://api.tavily.com/search",
        ] {
            assert!(super::authorized(&endpoints, url).is_err());
        }
        assert!(super::authorized(&[], "https://api.tavily.com/search").is_err());
    }

    #[test]
    fn accepts_only_get_and_post_methods() {
        assert!(matches!(reqwest::Method::GET.as_str(), "GET" | "POST"));
        assert!(matches!(reqwest::Method::POST.as_str(), "GET" | "POST"));
        assert!(!matches!(reqwest::Method::DELETE.as_str(), "GET" | "POST"));
    }
}
