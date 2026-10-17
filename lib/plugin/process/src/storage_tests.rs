use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use axum::{
    Router,
    body::{Body, Bytes},
    extract::RawQuery,
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::json;

use super::*;

#[tokio::test]
async fn object_client_bounds_downloads_distinguishes_failures_and_rejects_redirects() -> Result<()>
{
    let root = tempfile::tempdir()?;
    let socket = root.path().join("broker.sock");
    let listener = tokio::net::UnixListener::bind(&socket)?;
    let redirected = Arc::new(AtomicBool::new(false));
    let redirect_probe = redirected.clone();
    let router = Router::new().route("/storage", get(response)).route(
        "/leaked",
        get(move || {
            let redirect_probe = redirect_probe.clone();
            async move {
                redirect_probe.store(true, Ordering::SeqCst);
                "leaked"
            }
        }),
    );
    let mut jobs = tokio::task::JoinSet::new();
    jobs.spawn(async move { axum::serve(listener, router).await });
    let mut configuration = json!({
        "abi_version":2,"tenant_id":"tenant","ingress_token":"a".repeat(32),
        "broker_socket":socket,"endpoints":[],"services":[],"storage":true,
    });
    let host = Host::from_bytes(
        &serde_json::to_vec(&configuration)?,
        root.path().join("service.sock"),
    )?;
    let storage = host.storage()?;
    assert_eq!(storage.read("missing").await?, None);
    assert_eq!(storage.read("empty").await?, Some(vec![]));
    for key in [
        "failure",
        "redirect",
        "oversized-length",
        "oversized-stream",
    ] {
        assert!(storage.read(key).await.is_err(), "{key}");
    }
    assert!(!redirected.load(Ordering::SeqCst));
    assert!(
        storage
            .read(&"a".repeat(MAX_OBJECT_KEY_BYTES + 1))
            .await
            .is_err()
    );
    configuration["storage"] = json!(false);
    assert!(
        Host::from_bytes(
            &serde_json::to_vec(&configuration)?,
            root.path().join("service.sock")
        )?
        .storage()
        .is_err()
    );
    jobs.shutdown().await;
    Ok(())
}

async fn response(RawQuery(query): RawQuery, headers: HeaderMap) -> Response {
    assert_eq!(headers["x-aio-token"], "a".repeat(32));
    match query.as_deref() {
        Some("key=missing") => StatusCode::NOT_FOUND.into_response(),
        Some("key=empty") => Vec::<u8>::new().into_response(),
        Some("key=failure") => (StatusCode::SERVICE_UNAVAILABLE, "private-error").into_response(),
        Some("key=redirect") => (StatusCode::FOUND, [("location", "/leaked")]).into_response(),
        Some("key=oversized-length") => vec![0; MAX_OBJECT_BYTES + 1].into_response(),
        Some("key=oversized-stream") => {
            let chunks =
                (0..17).map(|_| Ok::<_, std::io::Error>(Bytes::from(vec![0; 1024 * 1024])));
            Body::from_stream(futures_util::stream::iter(chunks)).into_response()
        }
        _ => StatusCode::BAD_REQUEST.into_response(),
    }
}
