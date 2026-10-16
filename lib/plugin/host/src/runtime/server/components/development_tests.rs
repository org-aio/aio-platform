use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    body::Bytes,
    routing::{get, post},
};
use az_plugin_bundle::Bundle;
use az_plugin_contract::RequestContext;
use az_plugin_development::DevArtifact;
use serde_json::json;
use uuid::Uuid;

use super::{Components, execution::Request};

async fn echo(components: &Components, source: Uuid, digest: &str) -> Result<Vec<u8>> {
    Ok(components
        .handle(
            source,
            "development",
            digest,
            Request {
                method: "POST".into(),
                path: "/echo".into(),
                query: None,
                headers: vec![],
                body: b"echo".to_vec(),
            },
            RequestContext {
                tenant_id: Some("development".into()),
                user_id: Some("developer".into()),
                ..Default::default()
            },
        )
        .await?
        .body)
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL 和真实 WIT Component"]
async fn development_switch_preserves_cli_ownership_and_refreshes_frontend() -> Result<()> {
    let database = std::env::var("AIO_COMPONENT_TEST_DATABASE_URL")?;
    let url = reqwest::Url::parse(&database)?;
    ensure!(
        matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
            && url.path().ends_with("component_market_test"),
        "只接受本机独立组件测试库"
    );
    let root = tempfile::tempdir()?;
    let state = crate::runtime::server::RuntimeState::isolated_component_test(
        Arc::new(super::tests::TestIdentity),
        &database,
        "http://127.0.0.1",
        root.path(),
    )
    .await?;
    let components = state.components()?;
    let git = format!("https://github.com/example/dev-{}.git", Uuid::new_v4());
    let wasm_root = root.path().join("wasm");
    let first = super::tests::package(&wasm_root, &git, None, "1.0.0")?;
    let source = Uuid::new_v5(&Uuid::NAMESPACE_URL, git.as_bytes());
    let mut artifact = DevArtifact {
        workspace: root.path().into(),
        source: git.clone(),
        content_digest: first.digest.clone(),
        frontend: wasm_root.join("frontend"),
        backend: wasm_root.join("plugin.wasm"),
        backend_digest: "a".repeat(64),
        endpoint: None,
        generation: 1,
    };
    components
        .activate_development(&artifact, Arc::new(first.verify()?), &git)
        .await?;
    assert_eq!(echo(&components, source, &first.digest).await?, b"echo");
    std::fs::write(
        wasm_root.join("frontend/index.html"),
        "<html>changed frontend</html>",
    )?;
    let second = Bundle::from_directory(
        &wasm_root,
        "aio-plugin.toml",
        git.clone(),
        "a".repeat(40),
        "1.0.0".into(),
    )?;
    artifact.generation += 1;
    artifact.content_digest = second.digest.clone();
    components
        .activate_development(&artifact, Arc::new(second.verify()?), &git)
        .await?;
    assert!(echo(&components, source, &first.digest).await.is_err());
    assert_eq!(echo(&components, source, &second.digest).await?, b"echo");
    assert_eq!(
        components
            .bundle(source, "development")
            .await?
            .frontend("index.html"),
        Some(b"<html>changed frontend</html>".as_slice())
    );

    let process =
        super::process_publication_tests::package(&root.path().join("process"), &git, "2.0.0")?;
    artifact.backend_digest = "b".repeat(64);
    artifact.content_digest = process.digest.clone();
    artifact.generation += 1;
    let launch = components
        .prepare_development(&artifact, Arc::new(process.verify()?))
        .await?;
    let socket = launch.socket.context("未返回开发 socket")?;
    let broker = socket
        .parent()
        .context("缺少运行目录")?
        .parent()
        .context("缺少开发目录")?
        .join("broker/gateway.sock");
    let listener = tokio::net::UnixListener::bind(&socket)?;
    let router = Router::new()
        .route("/aio/describe", get(|| async { Json(json!({"label":"Development","pages":[{"id":"fixture","label":"Fixture","entry":"index.html","scene":null,"menu_path":[],"permission":null,"surface":"workspace"}]})) }))
        .route("/echo", post(|body: Bytes| async move { body }));
    // 此监听任务代表 CLI 所有的进程，宿主只能关闭自己的 broker 与转发。
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let result = async {
        components
            .activate_development(&artifact, Arc::new(process.verify()?), &git)
            .await?;
        assert_eq!(echo(&components, source, &process.digest).await?, b"echo");
        tokio::net::UnixStream::connect(&broker).await?;
        artifact.backend_digest = "a".repeat(64);
        artifact.content_digest = second.digest.clone();
        artifact.generation += 1;
        components
            .activate_development(&artifact, Arc::new(second.verify()?), &git)
            .await?;
        assert_eq!(echo(&components, source, &second.digest).await?, b"echo");
        assert!(!server.is_finished());
        assert!(tokio::net::UnixStream::connect(&broker).await.is_err());

        let pending = components
            .prepare_development(&artifact, Arc::new(process.verify()?))
            .await?;
        let pending = pending.socket.context("未返回候选 socket")?;
        let pending_broker = pending
            .parent()
            .context("缺少运行目录")?
            .parent()
            .context("缺少候选目录")?
            .join("broker/gateway.sock");
        tokio::net::UnixStream::connect(&pending_broker).await?;
        components.discard_development_candidate().await?;
        assert!(
            tokio::net::UnixStream::connect(&pending_broker)
                .await
                .is_err()
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    let cleanup = state.shutdown().await;
    assert!(!server.is_finished());
    server.abort();
    let _ = server.await;
    result?;
    cleanup?;
    Ok(())
}
