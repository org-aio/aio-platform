use anyhow::Result;
use axum::{Extension, Router, body::Body, http::Request, routing::get};
use serde_json::json;
use tower::ServiceExt;

use super::{Caller, Host};

fn config() -> serde_json::Value {
    json!({"abi_version":2,"tenant_id":"tenant-a","ingress_token":"a".repeat(32),
        "database_url":null,"encryption_key":null,"broker_socket":"/tmp/broker.sock",
        "endpoints":[],"services":[]})
}

#[test]
fn rejects_invalid_host_bindings() -> Result<()> {
    Host::from_bytes(&serde_json::to_vec(&config())?, "/tmp/service.sock".into())?;
    for (key, value) in [
        ("abi_version", json!(1)),
        ("tenant_id", json!("")),
        ("tenant_id", json!("x".repeat(129))),
        ("ingress_token", json!("short")),
        ("broker_socket", json!("relative.sock")),
    ] {
        let mut candidate = config();
        candidate[key] = value;
        assert!(
            Host::from_bytes(&serde_json::to_vec(&candidate)?, "/tmp/service.sock".into()).is_err()
        );
    }
    assert!(Host::from_bytes(&serde_json::to_vec(&config())?, "relative.sock".into()).is_err());
    assert!(Host::from_bytes(&vec![b' '; 65537], "/tmp/service.sock".into()).is_err());
    Ok(())
}

#[tokio::test]
async fn guards_identity_before_business_handler() -> Result<()> {
    let host = Host::from_bytes(&serde_json::to_vec(&config())?, "/tmp/service.sock".into())?;
    let router = host.protect(Router::new().route(
        "/",
        get(|Extension(caller): Extension<Caller>| async move {
            format!(
                "{}:{}:{}",
                caller.tenant_id, caller.user_id, caller.context_id
            )
        }),
    ));
    for (token, tenant, user, context, status) in [
        ("a".repeat(32), "tenant-a", "user", "context", 200),
        ("b".repeat(32), "tenant-a", "user", "context", 401),
        ("a".repeat(32), "tenant-b", "user", "context", 403),
        ("a".repeat(32), "tenant-a", "", "context", 401),
        ("a".repeat(32), "tenant-a", "user", "", 401),
    ] {
        let request = Request::builder()
            .uri("/")
            .header("x-aio-token", token)
            .header("x-aio-tenant-id", tenant)
            .header("x-aio-user-id", user)
            .header("x-aio-context", context)
            .body(Body::empty())?;
        let response = router.clone().oneshot(request).await?;
        assert_eq!(response.status(), status);
        if status == 200 {
            assert_eq!(
                axum::body::to_bytes(response.into_body(), 1024)
                    .await?
                    .as_ref(),
                b"tenant-a:user:context"
            );
        }
    }
    let request = Request::builder()
        .uri("/")
        .header("x-aio-token", "a".repeat(32))
        .header("x-aio-tenant-id", "tenant-a")
        .header("x-aio-user-id", "user")
        .header("x-aio-user-id", "admin")
        .header("x-aio-context", "context")
        .body(Body::empty())?;
    assert_eq!(router.oneshot(request).await?.status(), 401);
    Ok(())
}

#[tokio::test]
async fn public_ingress_has_only_the_bound_tenant_and_no_user_authority() -> Result<()> {
    let mut configuration = config();
    configuration["public_routes"] =
        json!([{"path":"/i/{token}","content_types":["image/png"],"max_bytes":1048576}]);
    let host = Host::from_bytes(
        &serde_json::to_vec(&configuration)?,
        "/tmp/service.sock".into(),
    )?;
    let app = host.protect_public(Router::new().route(
        "/i/{token}",
        get(|Extension(caller): Extension<super::PublicCaller>| async move { caller.tenant_id }),
    ));
    for (method, path, extra, expected) in [
        ("GET", "/i/abc", None, 200),
        ("HEAD", "/i/abc", None, 200),
        ("POST", "/i/abc", None, 403),
        ("GET", "/i/%252e", None, 403),
        ("GET", "/i/abc", Some(("x-aio-user-id", "reader")), 403),
        ("GET", "/i/abc", Some(("x-aio-context", "context")), 403),
        (
            "GET",
            "/i/abc",
            Some(("authorization", "Bearer secret")),
            403,
        ),
        ("GET", "/i/abc", Some(("cookie", "session=secret")), 403),
        ("GET", "/i/abc", Some(("x-aio-tenant-id", "tenant-b")), 403),
        ("GET", "/i/abc", Some(("x-aio-public", "1")), 403),
    ] {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("x-aio-token", "a".repeat(32))
            .header("x-aio-tenant-id", "tenant-a")
            .header("x-aio-public", "1");
        if let Some((name, value)) = extra {
            request = request.header(name, value);
        }
        let response = app.clone().oneshot(request.body(Body::empty())?).await?;
        assert_eq!(response.status(), expected, "{method} {path} {extra:?}");
        if method == "GET" && expected == 200 {
            assert_eq!(
                axum::body::to_bytes(response.into_body(), 1024)
                    .await?
                    .as_ref(),
                b"tenant-a"
            );
        }
    }
    assert_eq!(
        app.oneshot(Request::builder().uri("/i/abc").body(Body::empty())?)
            .await?
            .status(),
        403
    );
    Ok(())
}
