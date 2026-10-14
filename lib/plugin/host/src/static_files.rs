use std::path::PathBuf;

use axum::{
    Router,
    extract::Request,
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use tower_http::services::{ServeDir, ServeFile};

pub fn application(root: PathBuf) -> Router {
    let index = ServeFile::new(root.join("index.html"));
    // Brotli 优先：同一份 Wasm 在构建期预压缩为 .br 后比 gzip 小约三成，
    // 直接减少公网首屏的传输体积。缺少 .br 的文件自动回退 gzip/原始字节。
    Router::new()
        .fallback_service(
            ServeDir::new(root)
                .precompressed_br()
                .precompressed_gzip()
                .fallback(index),
        )
        .layer(middleware::from_fn(cache_headers))
}

async fn cache_headers(request: Request, next: Next) -> Response {
    let immutable = fingerprinted_asset(request.uri().path());
    if immutable && request.method() == Method::OPTIONS {
        let mut response = StatusCode::NO_CONTENT.into_response();
        let headers = response.headers_mut();
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("*"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, HEAD, OPTIONS"),
        );
        headers.insert(
            "access-control-allow-private-network",
            HeaderValue::from_static("true"),
        );
        return response;
    }
    let mut response = next.run(request).await;
    let html = response
        .headers()
        .get(header::CONTENT_TYPE)
        .is_some_and(|value| value.as_bytes().starts_with(b"text/html"));
    if html {
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
    }
    let cache = if immutable && !html && response.status().is_success() {
        // 指纹资产公开且不含账户数据，允许公共入口无 Cookie 读取可信局域网副本。
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("*"),
        );
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
        .headers_mut()
        .append(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    response
}

pub(crate) fn fingerprinted_asset(path: &str) -> bool {
    path.starts_with("/assets/")
        && path.rsplit('/').next().is_some_and(|file| {
            file.split_once("-dxh")
                .and_then(|(_, tail)| tail.split_once('.'))
                .is_some_and(|(hash, _)| {
                    hash.len() >= 8 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
        })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    #[test]
    fn only_content_fingerprints_are_immutable() {
        assert!(super::fingerprinted_asset(
            "/assets/aio-idea_bg-dxh15e2215685c7a54.wasm"
        ));
        assert!(!super::fingerprinted_asset("/assets/current.wasm"));
        assert!(!super::fingerprinted_asset(
            "/api/runtime/frontend/assets/token/app-dxh12345678.js"
        ));
        assert!(!super::fingerprinted_asset("/assets/app-dxhnotahash.js"));
    }

    #[tokio::test]
    async fn precompressed_assets_are_cached_without_caching_html() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        std::fs::create_dir(root.path().join("assets"))?;
        std::fs::write(
            root.path().join("index.html"),
            "<!doctype html><title>AIO IDEA</title>",
        )?;
        let name = "assets/app-dxh12345678.wasm";
        let bytes = b"static wasm fixture";
        std::fs::write(root.path().join(name), bytes)?;
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        gzip.write_all(bytes)?;
        let compressed = gzip.finish()?;
        std::fs::write(root.path().join(format!("{name}.gz")), &compressed)?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let router = super::application(root.path().to_owned());
        let server = tokio::spawn(async move { axum::serve(listener, router).await });
        let client = reqwest::Client::new();
        let result = async {
            let response = client
                .get(format!("{base}/{name}"))
                .header("accept-encoding", "gzip")
                .send()
                .await?;
            assert_eq!(response.headers()["content-encoding"], "gzip");
            assert_eq!(response.headers()["vary"], "Accept-Encoding");
            assert_eq!(response.headers()["access-control-allow-origin"], "*");
            assert!(
                response
                    .headers()
                    .get("access-control-allow-credentials")
                    .is_none()
            );
            assert_eq!(
                response.headers()["cache-control"],
                "public, max-age=31536000, immutable"
            );
            assert_eq!(response.bytes().await?.as_ref(), compressed);
            let preflight = client
                .request(reqwest::Method::OPTIONS, format!("{base}/{name}"))
                .header("origin", "https://public.example")
                .header("access-control-request-private-network", "true")
                .send()
                .await?;
            assert_eq!(preflight.status(), 204);
            assert_eq!(
                preflight.headers()["access-control-allow-private-network"],
                "true"
            );
            assert_eq!(
                preflight.headers()["access-control-allow-methods"],
                "GET, HEAD, OPTIONS"
            );
            for path in ["/", "/assets/missing-dxh12345678.js"] {
                let response = client.get(format!("{base}{path}")).send().await?;
                assert!(
                    response
                        .headers()
                        .get("access-control-allow-origin")
                        .is_none()
                );
                assert_eq!(response.headers()["cache-control"], "no-cache");
                assert_eq!(
                    response.headers()["content-type"],
                    "text/html; charset=utf-8"
                );
            }
            anyhow::Ok(())
        }
        .await;
        server.abort();
        let _ = server.await;
        result
    }

    #[tokio::test]
    async fn brotli_precompressed_assets_win_over_gzip() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        std::fs::create_dir(root.path().join("assets"))?;
        std::fs::write(
            root.path().join("index.html"),
            "<!doctype html><title>AIO IDEA</title>",
        )?;
        let name = "assets/app-dxh12345678.wasm";
        // 构建期预压缩的 .br 与 .gz 副本；.br 更小，必须优先交付。
        let bytes = "static wasm fixture ".repeat(64).into_bytes();
        // 与构建期 node zlib brotli 质量 11 的产物一致，避免为测试引入编码器依赖。
        let brotli: Vec<u8> = [
            0x1b, 0xff, 0x04, 0xf8, 0x05, 0xea, 0x64, 0xa9, 0x5e, 0xae, 0x1f, 0xd8, 0x04, 0x05,
            0x46, 0x11, 0xe2, 0x60, 0x8d, 0x70, 0x88, 0x31, 0x85, 0x53, 0x27, 0x1e, 0x3f, 0x5b,
            0x06,
        ]
        .to_vec();
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        gzip.write_all(&bytes)?;
        let gzip = gzip.finish()?;
        assert!(brotli.len() < gzip.len());
        std::fs::write(root.path().join(name), &bytes)?;
        std::fs::write(root.path().join(format!("{name}.br")), &brotli)?;
        std::fs::write(root.path().join(format!("{name}.gz")), &gzip)?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let router = super::application(root.path().to_owned());
        let server = tokio::spawn(async move { axum::serve(listener, router).await });
        let client = reqwest::Client::new();
        let result = async {
            let response = client
                .get(format!("{base}/{name}"))
                .header("accept-encoding", "br, gzip")
                .send()
                .await?;
            assert_eq!(response.headers()["content-encoding"], "br");
            assert_eq!(response.headers()["vary"], "Accept-Encoding");
            assert_eq!(response.bytes().await?.as_ref(), brotli);
            // 只声明 gzip 的客户端仍拿到 gzip 副本。
            let response = client
                .get(format!("{base}/{name}"))
                .header("accept-encoding", "gzip")
                .send()
                .await?;
            assert_eq!(response.headers()["content-encoding"], "gzip");
            assert_eq!(response.bytes().await?.as_ref(), gzip);
            // 缺少 .br 的资产回退 gzip，不影响其它指纹资产。
            let other = "assets/other-dxh87654321.js";
            std::fs::write(root.path().join(other), b"console.log(1)")?;
            let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
            gzip.write_all(b"console.log(1)")?;
            std::fs::write(root.path().join(format!("{other}.gz")), gzip.finish()?)?;
            let response = client
                .get(format!("{base}/{other}"))
                .header("accept-encoding", "br, gzip")
                .send()
                .await?;
            assert_eq!(response.headers()["content-encoding"], "gzip");
            anyhow::Ok(())
        }
        .await;
        server.abort();
        let _ = server.await;
        result
    }
}
