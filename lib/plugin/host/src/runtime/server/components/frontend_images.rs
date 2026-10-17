//! 管理文档的图片传输：站内图片复用已授权处理器，外部图片仅允许受限公网 HTTPS。

use super::frontend_management::interaction;
use crate::runtime::server::{RuntimeState, http_error::RuntimeError, remote_access};
use anyhow::{Context, Result, ensure};
use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, header},
    response::Response,
};
use serde::Deserialize;
use std::time::Duration;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ImageRequest {
    url: String,
}

pub(super) async fn image(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(token): Path<String>,
    Json(request): Json<ImageRequest>,
) -> Result<Response, RuntimeError> {
    let _permit = state.frontend.request_slot()?;
    let access = interaction(&state, &headers, &token).await?;
    let mut response = match source(&request.url)? {
        ImageSource::Published => {
            let output = super::super::management_dispatch::invoke(
                access.state,
                access.scope,
                access.services,
                false,
                az_plugin_contract::transport::Request {
                    method: "GET".into(),
                    path: request.url,
                    query: None,
                    headers: vec![],
                    body: vec![],
                },
            )
            .await?;
            let mut response = Response::new(Body::from(output.body));
            *response.status_mut() = axum::http::StatusCode::from_u16(output.status)?;
            for header in output.headers {
                response.headers_mut().insert(
                    axum::http::HeaderName::from_bytes(header.name.as_bytes())?,
                    HeaderValue::from_str(&header.value)?,
                );
            }
            response
        }
        ImageSource::Public(url) => {
            tokio::time::timeout(Duration::from_secs(15), remote_image(url))
                .await
                .context("文档图片读取超时")??
        }
    };
    // 网络等待期间可能已退出登录、切换租户、撤权或升级，返回前再次检查原挂载。
    interaction(&state, &headers, &token).await?;
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox; default-src 'none'; frame-ancestors 'none'"),
    );
    Ok(response)
}

enum ImageSource {
    Published,
    Public(reqwest::Url),
}

fn source(input: &str) -> Result<ImageSource> {
    ensure!(
        input.len() <= 4096 && !input.chars().any(char::is_control),
        "图片地址无效"
    );
    if input.starts_with("/api/runtime/marketplace/") {
        super::super::management_dispatch::document_image_path(input)?;
        return Ok(ImageSource::Published);
    }
    let url = reqwest::Url::parse(input).context("图片地址必须是公开 HTTPS 或已发布文档路径")?;
    validate_public_url(&url)?;
    Ok(ImageSource::Public(url))
}

fn validate_public_url(url: &reqwest::Url) -> Result<()> {
    ensure!(
        url.scheme() == "https"
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "图片仅接受无凭据的公网 HTTPS 地址"
    );
    Ok(())
}

async fn remote_image(mut url: reqwest::Url) -> Result<Response> {
    for redirect in 0..=3 {
        validate_public_url(&url)?;
        let resolved = remote_access::validate_public_remote(url.as_str()).await?;
        // 绑定已检查的 DNS 地址，不使用本机代理、Cookie、仓库或发布凭据。
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .resolve(&resolved.host, resolved.socket)
            .build()?;
        let mut incoming = client
            .get(url.clone())
            .header(header::ACCEPT, "image/*")
            .send()
            .await?;
        if incoming.status().is_redirection() {
            ensure!(redirect < 3, "图片重定向次数超过限制");
            let location = incoming
                .headers()
                .get(header::LOCATION)
                .context("图片重定向缺少地址")?
                .to_str()?;
            url = url.join(location)?;
            continue;
        }
        ensure!(
            incoming.status().is_success(),
            "图片读取失败: HTTP {}",
            incoming.status()
        );
        let content_type = incoming
            .headers()
            .get(header::CONTENT_TYPE)
            .context("图片类型缺失")?
            .to_str()?
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        ensure!(
            [
                "image/png",
                "image/jpeg",
                "image/gif",
                "image/webp",
                "image/avif",
                "image/svg+xml"
            ]
            .contains(&content_type.as_str()),
            "远程内容不是支持的图片"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = incoming.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= 10 * 1024 * 1024,
                "图片超过 10 MiB"
            );
            bytes.extend_from_slice(&chunk);
        }
        let mut response = Response::new(Body::from(bytes));
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, HeaderValue::from_str(&content_type)?);
        return Ok(response);
    }
    anyhow::bail!("图片重定向次数超过限制")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_sources_preserve_unicode_and_reject_credentials_and_traversal() -> Result<()> {
        let prefix = format!("/api/runtime/marketplace/{}/images/", "a".repeat(64));
        assert!(matches!(
            source(&format!("{prefix}%E5%9B%BE%E7%89%87.png"))?,
            ImageSource::Published
        ));
        for invalid in ["../secret", "%2e%2e/secret", "%2Fetc/passwd"] {
            assert!(source(&format!("{prefix}{invalid}")).is_err());
        }
        for invalid in [
            "/api/identity/me",
            "//outside.test/image.png",
            "file:///etc/passwd",
            "http://outside.test/image.png",
            "https://user:password@outside.test/a.png",
            "https://outside.test:444/a.png",
        ] {
            assert!(source(invalid).is_err());
        }
        Ok(())
    }

    #[tokio::test]
    async fn image_fetch_rejects_local_targets_before_connection() -> Result<()> {
        for url in [
            "https://127.0.0.1/a.png",
            "https://localhost/a.png",
            "https://10.0.0.1/a.png",
            "https://169.254.169.254/a.png",
            "https://[::1]/a.png",
        ] {
            assert!(remote_image(url.parse()?).await.is_err(), "{url}");
        }
        Ok(())
    }
}
