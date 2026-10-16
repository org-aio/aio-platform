use std::{collections::HashSet, sync::Arc};

use anyhow::Result;
use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use tokio::sync::Mutex;

use super::{
    ProcessExecutor, ProcessInstance, ProcessManager, StartProcessRequest, StopProcessRequest,
};

#[derive(Clone, Default)]
struct Supervisor {
    running: Arc<Mutex<HashSet<String>>>,
    stops: Arc<Mutex<Vec<String>>>,
    fail: Arc<Mutex<bool>>,
}

async fn start(
    State(state): State<Supervisor>,
    Json(request): Json<StartProcessRequest>,
) -> Json<ProcessInstance> {
    let created = state.running.lock().await.insert(request.source_id.clone());
    Json(ProcessInstance {
        instance_id: request.source_id,
        endpoint: "http://172.29.0.2:8080".into(),
        created,
    })
}

async fn stop(
    State(state): State<Supervisor>,
    Json(request): Json<StopProcessRequest>,
) -> StatusCode {
    state.stops.lock().await.push(request.instance_id.clone());
    if *state.fail.lock().await && request.instance_id == "failed" {
        return StatusCode::INTERNAL_SERVER_ERROR;
    }
    state.running.lock().await.remove(&request.instance_id);
    StatusCode::NO_CONTENT
}

#[tokio::test]
async fn shutdown_only_stops_owned_processes_and_attempts_all_after_failure() -> Result<()> {
    let root = tempfile::tempdir()?;
    let socket = root.path().join("supervisor.sock");
    let listener = tokio::net::UnixListener::bind(&socket)?;
    let supervisor = Supervisor::default();
    supervisor.running.lock().await.insert("existing".into());
    *supervisor.fail.lock().await = true;
    let router = Router::new()
        .route("/instances/start", post(start))
        .route("/instances/stop", post(stop))
        .with_state(supervisor.clone());
    let mut servers = tokio::task::JoinSet::new();
    servers.spawn(async move { axum::serve(listener, router).await });
    let client = reqwest::Client::builder()
        .unix_socket(socket)
        .no_proxy()
        .build()?;
    let manager = ProcessManager {
        local: false,
        owned: Default::default(),
        supervisor: client.clone(),
        runtime: client,
    };
    for source in ["existing", "created", "failed"] {
        manager.start("tenant", source, "revision").await?;
    }
    assert!(manager.shutdown().await.is_err());
    assert_eq!(*supervisor.stops.lock().await, ["failed", "created"]);
    assert_eq!(
        *supervisor.running.lock().await,
        HashSet::from(["existing".to_owned(), "failed".to_owned()])
    );
    *supervisor.fail.lock().await = false;
    manager.shutdown().await?;
    assert_eq!(
        *supervisor.stops.lock().await,
        ["failed", "created", "failed"]
    );
    assert_eq!(
        *supervisor.running.lock().await,
        HashSet::from(["existing".to_owned()])
    );
    Ok(())
}
