use super::RuntimeState;
use crate::runtime::RuntimeResponse;
use anyhow::{Context, Result, ensure};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, HeaderValue, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;

/// 由部署配置提供受信任的局域网来源，业务插件不维护另一份入口清单。
#[derive(Clone, Serialize)]
pub(crate) struct TransportOrigins {
    pub public_origin: String,
    pub lan_origins: Vec<String>,
}

impl TransportOrigins {
    pub fn from_env(public_origin: &str) -> Result<Self> {
        Self::new(
            public_origin,
            &std::env::var("AIO_LAN_ORIGINS").unwrap_or_default(),
        )
    }

    fn new(public_origin: &str, configured: &str) -> Result<Self> {
        let public_origin = super::frontend_document::public_origin(public_origin)?;
        let mut lan_origins = Vec::new();
        for value in configured
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let value = super::frontend_document::public_origin(value)
                .context("AIO_LAN_ORIGINS 必须为可信 HTTPS 来源")?;
            ensure!(
                value.starts_with("https://"),
                "局域网生产入口必须使用 HTTPS"
            );
            if value != public_origin && !lan_origins.contains(&value) {
                lan_origins.push(value);
            }
        }
        ensure!(lan_origins.len() <= 4, "局域网入口最多四个");
        Ok(Self {
            public_origin,
            lan_origins,
        })
    }

    pub fn request_origin(&self, headers: &HeaderMap) -> &str {
        let host = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok());
        // 只接受管理员声明的来源；不信任任意 Host 或转发来源头。
        self.lan_origins
            .iter()
            .find(|origin| {
                reqwest::Url::parse(origin).ok().is_some_and(|url| {
                    let authority = url
                        .port()
                        .map(|port| format!("{}:{port}", url.host_str().unwrap_or_default()))
                        .unwrap_or_else(|| url.host_str().unwrap_or_default().to_owned());
                    authority == host.unwrap_or_default()
                })
            })
            .map(String::as_str)
            .unwrap_or(&self.public_origin)
    }

    pub fn accepts(&self, origin: &str) -> bool {
        origin == self.public_origin || self.lan_origins.iter().any(|value| value == origin)
    }
}

impl RuntimeState {
    /// 产品只增加部署声明的 HTTPS 来源，仍由各插件限制具体资产与通道路径。
    pub fn frame_policy(&self) -> String {
        format!(
            "frame-src 'self' {}; object-src 'none'",
            self.transport.lan_origins.join(" ")
        )
    }
}

pub(super) async fn discover(State(state): State<RuntimeState>) -> Response {
    let mut response = Json(RuntimeResponse {
        data: state.transport.clone(),
    })
    .into_response();
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response
}

// 本地网络预检只声明访问方式，正式请求仍校验既有的设备或挂载授权。
pub(crate) async fn preflight() -> Response {
    let mut response = axum::http::StatusCode::NO_CONTENT.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("content-type"),
    );
    headers.insert(
        "access-control-allow-private-network",
        HeaderValue::from_static("true"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_declared_https_origins_can_supply_asset_prefixes() -> Result<()> {
        let transport = TransportOrigins::new(
            "https://public.example",
            "https://lan.example:3443,https://lan.example:3443",
        )?;
        assert_eq!(transport.lan_origins.len(), 1);
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("lan.example:3443"));
        assert_eq!(
            transport.request_origin(&headers),
            "https://lan.example:3443"
        );
        headers.insert(header::HOST, HeaderValue::from_static("attacker.example"));
        headers.insert(
            "x-forwarded-host",
            HeaderValue::from_static("lan.example:3443"),
        );
        assert_eq!(transport.request_origin(&headers), "https://public.example");
        assert!(transport.accepts("https://lan.example:3443"));
        assert!(!transport.accepts("https://attacker.example"));
        Ok(())
    }

    #[test]
    fn plaintext_credentials_paths_and_excess_candidates_are_rejected() {
        for value in [
            "http://127.0.0.1:3443",
            "http://192.168.1.2",
            "https://user:password@lan.example",
            "https://lan.example/path",
            "https://lan.example/?token=secret",
        ] {
            assert!(TransportOrigins::new("https://public.example", value).is_err());
        }
        assert!(TransportOrigins::new("https://public.example", "https://a.example,https://b.example,https://c.example,https://d.example,https://e.example").is_err());
    }
}
