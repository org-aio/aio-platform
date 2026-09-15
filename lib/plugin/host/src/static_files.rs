use std::path::PathBuf;

use axum::{
    Router,
    extract::Request,
    http::{HeaderValue, header},
    middleware::{self, Next},
    response::Response,
};
use tower_http::services::{ServeDir, ServeFile};

pub fn application(root: PathBuf) -> Router {
    let index = ServeFile::new(root.join("index.html"));
    Router::new()
        .fallback_service(ServeDir::new(root).precompressed_gzip().fallback(index))
        .layer(middleware::from_fn(cache_headers))
}

async fn cache_headers(request: Request, next: Next) -> Response {
    let immutable = fingerprinted_asset(request.uri().path());
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

fn fingerprinted_asset(path: &str) -> bool {
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
            assert_eq!(
                response.headers()["cache-control"],
                "public, max-age=31536000, immutable"
            );
            assert_eq!(response.bytes().await?.as_ref(), compressed);
            for path in ["/", "/assets/missing-dxh12345678.js"] {
                let response = client.get(format!("{base}{path}")).send().await?;
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
}
