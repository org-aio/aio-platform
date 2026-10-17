//! 真实 Unix HTTP 与进程 SDK 验证两种入口的身份分离和流式传输。
use super::*;
use axum::{Extension, Router, body::Body, routing::get};
use az_plugin_contract::{
    RequestContext,
    transport::{Header, Request},
};
use futures_util::{StreamExt, stream};

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn sidecar_resources_keep_public_and_authenticated_ingress_separate() -> Result<()> {
    let fixture = Fixture::open().await?;
    let source = Uuid::new_v4();
    let instance = fixture.start(source, "resource-tenant", false).await?;
    let socket = fixture.root.path().join("resource.sock");
    let sdk = Host::from_bytes(
        &serde_json::to_vec(&instance.configuration)?,
        socket.clone(),
    )?;
    assert_eq!(
        instance.configuration.public_base.as_deref(),
        Some(format!("/api/runtime/components/public/{source}/resource-tenant").as_str())
    );
    let public = sdk.protect_public(Router::new().route(
        "/i/{token}",
        get(
            |Extension(caller): Extension<az_plugin_process::PublicCaller>| async move {
                assert_eq!(caller.tenant_id, "resource-tenant");
                let body =
                    stream::iter((0..1600).map(|_| Ok::<_, std::io::Error>(vec![0xa5; 65536])));
                axum::response::Response::builder()
                    .header("content-type", "image/png")
                    .header("content-length", 104857600)
                    .header("set-cookie", "secret=bad")
                    .body(Body::from_stream(body))
                    .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)
            },
        ),
    ));
    let private = sdk.protect(Router::new().route(
        "/private",
        get(
            |Extension(caller): Extension<az_plugin_process::Caller>| async move {
                format!(
                    "{}:{}:{}",
                    caller.tenant_id, caller.user_id, caller.context_id
                )
            },
        ),
    ));
    let listener = tokio::net::UnixListener::bind(&socket)?;
    let service = tokio::spawn(async move { axum::serve(listener, public.merge(private)).await });
    let bundle = fixture
        .components
        .catalog
        .archive(&instance.start.revision)
        .await?
        .context("缺少整包")?;
    let bundle = Arc::new(Bundle::decode(&bundle)?.verify()?);
    let active = Arc::new(super::super::model::Instance {
        start: instance.start.clone(),
        bundle,
        token: instance.configuration.ingress_token,
        client: reqwest::Client::builder()
            .unix_socket(socket.clone())
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
        resources: Arc::new(super::super::model::Resources {
            jobs: tokio::sync::Mutex::new(instance.jobs),
            _socket_alias: None,
        }),
    });
    fixture
        .processes
        .instances
        .lock()
        .await
        .insert((source, "resource-tenant".into()), active.clone());
    let public_request = Request {
        method: "GET".into(),
        path: "/i/token".into(),
        query: None,
        headers: vec![Header {
            name: "x-aio-user-id".into(),
            value: "forged".into(),
        }],
        body: vec![],
    };
    let mut response = fixture
        .processes
        .handle_public(
            source,
            "resource-tenant",
            &active.start.revision,
            public_request.clone(),
        )
        .await?;
    assert_eq!(response.status, 200);
    assert!(!response.headers.iter().any(|h| h.name == "set-cookie"));
    let mut count = 0;
    while let Some(chunk) = response.body.next().await {
        let chunk = chunk?;
        assert!(chunk.iter().all(|byte| *byte == 0xa5));
        count += chunk.len();
    }
    assert_eq!(count, 104857600);
    let context = RequestContext {
        request_id: "resource-context".into(),
        session_id: Some("resource-session".into()),
        tenant_id: Some("resource-tenant".into()),
        user_id: Some("resource-user".into()),
    };
    let private_request = Request {
        method: "GET".into(),
        path: "/private".into(),
        query: None,
        headers: vec![],
        body: vec![],
    };
    let mut response = fixture
        .processes
        .handle_stream(
            source,
            "resource-tenant",
            &active.start.revision,
            private_request.clone(),
            context.clone(),
        )
        .await?;
    assert_eq!(response.status, 200);
    let mut bytes = Vec::new();
    while let Some(chunk) = response.body.next().await {
        bytes.extend(chunk?);
    }
    assert_eq!(bytes, b"resource-tenant:resource-user:resource-context");
    assert!(
        fixture
            .processes
            .handle_public(
                source,
                "resource-tenant",
                &active.start.revision,
                private_request.clone()
            )
            .await
            .is_err()
    );
    assert!(
        fixture
            .processes
            .handle_stream(
                source,
                "other",
                &active.start.revision,
                private_request.clone(),
                context.clone()
            )
            .await
            .is_err()
    );
    assert!(
        fixture
            .processes
            .handle_public(source, "resource-tenant", "stale", public_request)
            .await
            .is_err()
    );
    assert!(
        fixture
            .processes
            .handle_stream(
                source,
                "resource-tenant",
                &active.start.revision,
                Request {
                    method: "POST".into(),
                    ..private_request
                },
                context
            )
            .await
            .is_err()
    );
    fixture.processes.instances.lock().await.clear();
    active.resources.shutdown().await;
    service.abort();
    let _ = service.await;
    fixture.components.shutdown().await?;
    fixture.pool.close().await;
    Ok(())
}
