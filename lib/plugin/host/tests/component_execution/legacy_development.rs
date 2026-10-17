use std::{
    path::Path,
    sync::{Arc, atomic::Ordering},
};

use anyhow::{Context, Result, ensure};
use axum::{body::Body, http::Request};
use az_plugin_core::DynPlugin;
use az_plugin_development::{DevArtifact, DevHostSession, DevWorkspace};
use az_plugin_host::runtime::server::{RuntimeState, development, plugins::*};
use serde_json::json;
use tower::ServiceExt;

use super::{
    fixture,
    legacy_executor::{LegacyExecutor, LegacyPlugin},
};

fn artifact(workspace: &Path, valid: bool, generation: u64) -> Result<DevArtifact> {
    let backend = workspace.join("plugin.wasm");
    let frontend = workspace.join("web");
    let page = json!([{"id":"external","label":if valid {"External"} else {""},
        "icon":null,"scene":{"id":"tools","label":"Tools"},"required_permission":null,
        "body":{"kind":"frontend","entry":"index.html"}}]);
    std::fs::write(&backend, format!("executor-fixture\n{page}"))?;
    Ok(DevArtifact {
        workspace: workspace.to_owned(),
        source: az_plugin_development::source_identity(workspace)?,
        content_digest: az_plugin_development::artifact_digest(
            &workspace.join("aio-plugin.toml"),
            &frontend,
            &backend,
        )?,
        backend_digest: az_plugin_development::backend_digest(&backend)?,
        frontend,
        backend,
        endpoint: None,
        generation,
    })
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn development_snapshots_use_selected_executor_and_reject_bad_candidates_before_activation()
-> Result<()> {
    let database = fixture::database().await?;
    let root = tempfile::tempdir()?;
    let workspace = root.path().join("workspace");
    std::fs::create_dir_all(workspace.join("web"))?;
    std::fs::write(
        workspace.join("web/index.html"),
        "<!doctype html><title>External</title>",
    )?;
    std::fs::write(
        workspace.join("aio-plugin.toml"),
        "[plugin.runtime]\nkind='wasm-component'\nartifact='plugin.wasm'\n[plugin.frontend]\npath='web'\n[[plugin.subplugins]]\nid='external'\npages=['external']\n",
    )?;
    let first = artifact(&workspace, true, 1)?;
    let mut config = fixture::config(root.path(), &database, "http://127.0.0.1");
    config.development = Some(DevHostSession {
        database_url: database,
        root: root.path().to_owned(),
        port: 0,
        token: "fixture-dev".into(),
        workspaces: vec![DevWorkspace {
            path: workspace,
            source: first.source.clone(),
            version: "1.0.0".into(),
        }],
    });
    let selected: Vec<DynPlugin<RuntimeAssembly>> = vec![
        Arc::new(RepositoryPlugin),
        Arc::new(StorePlugin),
        Arc::new(DatabasePlugin),
        Arc::new(PostgresPlugin),
        Arc::new(LegacyPlugin::<1>),
    ];
    let state = RuntimeState::initialize(config, selected).await?;
    let router = development::router(state.clone())?;
    let response = router
        .clone()
        .oneshot(
            Request::post("/api/development/activate")
                .header("authorization", "Bearer fixture-dev")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&first)?))?,
        )
        .await?;
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 65536).await?;
    ensure!(
        status.is_success(),
        "开发激活失败: {}",
        String::from_utf8_lossy(&body)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body)?["revision"],
        first.content_digest
    );
    assert_eq!(
        state
            .service::<LegacyExecutor<1>>()?
            .validations
            .load(Ordering::SeqCst),
        2
    );

    let invalid = artifact(&first.workspace, false, 2)?;
    let response = router
        .oneshot(
            Request::post("/api/development/activate")
                .header("authorization", "Bearer fixture-dev")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&invalid)?))?,
        )
        .await?;
    assert!(!response.status().is_success());
    drop(response);
    assert!(
        !state
            .config
            .cache_root
            .join(&invalid.content_digest)
            .exists()
    );
    let pool = state.service::<sqlx::PgPool>()?;
    let revision:Option<String> = sqlx::query_scalar("SELECT r.revision FROM tenant_plugin_bindings b JOIN plugin_revisions r ON r.id=b.revision_id WHERE b.tenant_id='development' AND b.source_id=$1")
        .bind(uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL,first.source.as_bytes()).to_string())
        .fetch_optional(pool.as_ref()).await?;
    assert_eq!(
        revision.context("失败候选清除了已激活版本")?,
        first.content_digest
    );
    // 开发产物只存在于本测试工作区，不能留给其他测试的生产恢复流程。
    sqlx::query(
        "DELETE FROM tenant_plugin_bindings WHERE tenant_id='development' AND source_id=$1",
    )
    .bind(uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, first.source.as_bytes()).to_string())
    .execute(pool.as_ref())
    .await?;
    tokio::time::timeout(std::time::Duration::from_secs(10), state.shutdown()).await??;
    Ok(())
}
