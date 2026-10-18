mod document;
mod transport;

use anyhow::{Context, Result};
use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use az_plugin_contract::{CapabilityGrants, InvocationScope, RequestContext};
use az_plugin_runtime::{
    ComponentEngine, ComponentInstance, DatabaseProvisioner, InvocationResources,
    bindings::aio::plugin::transport::Phase,
};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

struct Preview {
    instance: Mutex<ComponentInstance>,
    assets: PathBuf,
    context: RequestContext,
    ticket: String,
    origin: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let component = std::fs::read(std::env::var("AIO_TEST_COMPONENT")?)?;
    let migration = std::fs::read_to_string(std::env::var("AIO_TEST_MIGRATION")?)?;
    let assets = std::fs::canonicalize(std::env::var("AIO_PREVIEW_ASSETS")?)?;
    let port: u16 = std::env::var("AIO_PREVIEW_PORT")
        .unwrap_or_else(|_| "4187".into())
        .parse()?;
    let mut random = [0; 32];
    getrandom::fill(&mut random).map_err(|error| anyhow::anyhow!("随机票据生成失败: {error}"))?;
    let ticket = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let source_id = format!("preview-{ticket}");
    let database = DatabaseProvisioner::connect(&std::env::var("AIO_TEST_DATABASE_URL")?)
        .await?
        .create(&source_id, "preview", &migration)
        .await?;
    let context = RequestContext {
        tenant_id: Some("preview".into()),
        user_id: Some("developer".into()),
        session_id: None,
        request_id: "preview".into(),
        permissions: Vec::new(),
    };
    let engine = ComponentEngine::new()?;
    let component = engine.compile(&component).await?;
    let mut instance = engine
        .instantiate(
            &component,
            InvocationScope {
                source_id,
                revision: "development".into(),
                context: context.clone(),
                grants: CapabilityGrants {
                    database: true,
                    ..Default::default()
                },
            },
            InvocationResources {
                database: Some(database),
                ..Default::default()
            },
        )
        .await?;
    instance.describe().await?;
    instance.lifecycle(Phase::Prepare).await?;
    instance.health().await?;
    instance.lifecycle(Phase::Activate).await?;
    let origin = format!("http://127.0.0.1:{port}");
    let state = Arc::new(Preview {
        instance: Mutex::new(instance),
        assets,
        context,
        ticket,
        origin: origin.clone(),
    });
    let app = Router::new()
        .route(
            "/favicon.ico",
            get(|| async { axum::http::StatusCode::NO_CONTENT }),
        )
        .route("/", get(document::shell))
        .route("/assets/{*path}", get(document::asset))
        .route("/bridge/guest.js", get(document::guest))
        .route("/bridge/host.mjs", get(document::host))
        .route("/invoke", post(transport::invoke))
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .context("预览端口被占用")?;
    println!("{origin}");
    axum::serve(listener, app).await?;
    Ok(())
}
