use super::*;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde_json::json;
use tokio::{net::UnixListener, task::JoinHandle};

struct Running {
    start: process::Start,
    server: JoinHandle<()>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.server.abort();
    }
}

#[derive(Default)]
struct Supervisor {
    running: Mutex<HashMap<String, Running>>,
    starts: Mutex<Vec<process::Start>>,
}

// 模拟同一租户只能运行一个后台实例，并通过真实 Unix socket 返回健康状态和描述。
async fn start(
    State(state): State<Arc<Supervisor>>,
    Json(request): Json<process::Start>,
) -> Result<StatusCode, StatusCode> {
    let mut running = state.running.lock().await;
    if running.values().any(|instance| {
        instance.start.source == request.source && instance.start.tenant == request.tenant
    }) {
        return Err(StatusCode::CONFLICT);
    }
    let root = std::env::var_os("AIO_PROCESS_ROOT").ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    let socket = PathBuf::from(root)
        .join(request.id())
        .join("runtime/service.sock");
    if socket.exists() {
        std::fs::remove_file(&socket).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    let listener = UnixListener::bind(socket).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route(
            "/aio/describe",
            get(|| async {
                Json(json!({"label":"Fixture","pages":[{
                    "id":"chat","label":"Chat","entry":"index.html","scene":null,
                    "menu_path":[],"permission":null,"surface":"workspace"
                }]}))
            }),
        );
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    state.starts.lock().await.push(request.clone());
    running.insert(
        request.id(),
        Running {
            start: request,
            server,
        },
    );
    Ok(StatusCode::NO_CONTENT)
}

async fn stop(
    State(state): State<Arc<Supervisor>>,
    Json(request): Json<process::Stop>,
) -> StatusCode {
    state.running.lock().await.remove(&request.id);
    StatusCode::NO_CONTENT
}

fn package(root: &Path, git: &str, version: &str) -> Result<Bundle> {
    std::fs::create_dir_all(root.join("web"))?;
    std::fs::write(root.join("web/index.html"), "<html>Fixture</html>")?;
    let mut binary = vec![0; 64];
    binary[..6].copy_from_slice(b"\x7fELF\x02\x01");
    binary[18] = 62;
    std::fs::write(root.join("server"), binary)?;
    std::fs::write(
        root.join("aio-plugin.toml"),
        format!(
            "schema_version=2\n[plugin.marketplace]\ntitle='Fixture'\nsummary='Process publication isolation'\nlicense='MIT'\n[plugin.runtime]\nartifact='server'\nhost_version='>=2026.9.21'\n[plugin.runtime.process]\nimage='sha256:{}'\n[plugin.frontend]\npath='web'\n",
            "a".repeat(64)
        ),
    )?;
    Bundle::from_directory(
        root,
        "aio-plugin.toml",
        git.into(),
        "a".repeat(40),
        version.into(),
    )
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL、AIO_PROCESS_ROOT 和监督器测试 socket"]
async fn publication_preserves_running_singleton_process() -> Result<()> {
    let database = std::env::var("AIO_COMPONENT_TEST_DATABASE_URL")?;
    let url = reqwest::Url::parse(&database)?;
    ensure!(
        matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
            && url.path().contains("component_market_test"),
        "只接受本机独立市场测试库"
    );
    let socket = std::env::var_os("AIO_PLUGIN_SUPERVISOR_SOCKET").context("缺少测试 socket")?;
    let listener = UnixListener::bind(socket)?;
    let supervisor = Arc::new(Supervisor::default());
    let app = Router::new()
        .route("/bundles/start", post(start))
        .route("/bundles/stop", post(stop))
        .with_state(supervisor.clone());
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let root = tempfile::tempdir()?;
    let pool = PgPool::connect(&database).await?;
    sqlx::raw_sql("CREATE TABLE IF NOT EXISTS plugin_sources(id TEXT PRIMARY KEY,git TEXT); CREATE TABLE IF NOT EXISTS tenant_plugin_bindings(tenant_id TEXT,source_id TEXT,enabled BOOLEAN);").execute(&pool).await?;
    let components = Components::open(
        pool,
        &database,
        &root.path().join("keys.json"),
        root.path().join("objects"),
        Arc::new(super::tests::TestIdentity),
    )
    .await?;
    let git = format!(
        "https://github.com/example/singleton-{}.git",
        Uuid::new_v4()
    );
    let first = package(root.path(), &git, "1.0.0")?;
    let source = components.publish(first.clone(), "First").await?;
    components.install("default", &git, None).await?;
    let original = process::Start {
        source,
        tenant: "default".into(),
        revision: first.digest.clone(),
    };

    // 重发相同整包和发布新版本均不能触碰已安装实例。
    components.publish(first.clone(), "Same").await?;
    let second = package(root.path(), &git, "2.0.0")?;
    components.publish(second, "Second").await?;
    assert_eq!(
        components
            .installed_bundle("default", source)
            .await?
            .context("实例未安装")?
            .digest,
        first.digest
    );
    let running = supervisor.running.lock().await;
    assert_eq!(running.len(), 1);
    assert!(running.contains_key(&original.id()));
    drop(running);
    assert_eq!(
        supervisor
            .starts
            .lock()
            .await
            .iter()
            .filter(|start| start.tenant == "default")
            .count(),
        1
    );
    let root = PathBuf::from(std::env::var_os("AIO_PROCESS_ROOT").context("缺少进程目录")?);
    let client = reqwest::Client::builder()
        .unix_socket(root.join(original.id()).join("runtime/service.sock"))
        .no_proxy()
        .build()?;
    assert!(
        client
            .get("http://localhost/health")
            .send()
            .await?
            .status()
            .is_success()
    );
    components.change("default", source, "uninstall").await?;
    server.abort();
    Ok(())
}
