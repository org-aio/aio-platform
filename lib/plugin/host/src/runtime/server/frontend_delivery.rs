use axum::http::{Extensions, HeaderMap, StatusCode, Version, header};
use tower_http::compression::{
    CompressionLayer,
    predicate::{Predicate, SizeAbove},
};

pub(super) fn compression() -> CompressionLayer<impl Predicate> {
    // 由源站压缩静态代码，保留 no-transform，禁止代理改写隔离文档。
    CompressionLayer::new().compress_when(SizeAbove::new(1024).and(
        |status: StatusCode, _: Version, headers: &HeaderMap, _: &Extensions| {
            status == StatusCode::OK
                && matches!(
                    headers
                        .get(header::CONTENT_TYPE)
                        .and_then(|value| value.to_str().ok()),
                    Some(
                        "application/wasm"
                            | "application/javascript"
                            | "text/javascript"
                            | "text/css"
                    )
                )
        },
    ))
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use axum::{Router, routing::get};
    use flate2::read::GzDecoder;

    use super::*;

    #[tokio::test]
    async fn static_assets_negotiate_lossless_compression_without_relaxing_isolation()
    -> anyhow::Result<()> {
        let bytes = b"\0asm".repeat(4096);
        let fixture = bytes.clone();
        let app = Router::new()
            .route(
                "/frontend.wasm",
                get(move || async move {
                    (
                        [
                            (header::CONTENT_TYPE, "application/wasm"),
                            (header::CACHE_CONTROL, "private, no-store, no-transform"),
                            (header::CONTENT_SECURITY_POLICY, "sandbox allow-scripts"),
                            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
                        ],
                        fixture,
                    )
                })
                .layer(compression()),
            )
            .route(
                "/index.html",
                get(|| async { ([(header::CONTENT_TYPE, "text/html")], "ticket".repeat(1024)) })
                    .layer(compression()),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move { axum::serve(listener, app).await });
        let result: anyhow::Result<()> = async {
            let client = reqwest::Client::new();
            for (accept, compressed) in [("gzip", true), ("gzip;q=0", false), ("identity", false)] {
                let response = client
                    .get(format!("http://{address}/frontend.wasm"))
                    .header(header::ACCEPT_ENCODING, accept)
                    .send()
                    .await?;
                assert_eq!(response.status(), StatusCode::OK);
                assert_eq!(
                    response.headers()[header::CACHE_CONTROL],
                    "private, no-store, no-transform"
                );
                assert_eq!(
                    response.headers()[header::CONTENT_SECURITY_POLICY],
                    "sandbox allow-scripts"
                );
                assert_eq!(response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
                assert_eq!(response.headers()[header::VARY], "accept-encoding");
                assert_eq!(
                    response.headers().contains_key(header::CONTENT_ENCODING),
                    compressed
                );
                let body = response.bytes().await?;
                if compressed {
                    assert!(body.len() < bytes.len() / 4);
                    let mut decoded = Vec::new();
                    GzDecoder::new(body.as_ref()).read_to_end(&mut decoded)?;
                    assert_eq!(decoded, bytes);
                } else {
                    assert_eq!(body.as_ref(), bytes);
                }
            }
            let document = client
                .get(format!("http://{address}/index.html"))
                .header(header::ACCEPT_ENCODING, "gzip")
                .send()
                .await?;
            assert!(!document.headers().contains_key(header::CONTENT_ENCODING));
            Ok(())
        }
        .await;
        server.abort();
        result
    }
}
