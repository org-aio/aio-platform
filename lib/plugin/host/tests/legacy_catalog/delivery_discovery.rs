//! 真 HTTP GitHub 替身与文件队列验证独立发现、取消和在线替换，不访问公网或数据库。
use super::delivery_http::{artifact, build_request, claim, selection, upload};
use super::*;
use axum::{
    Router,
    extract::{OriginalUri, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use az_plugin_delivery::{BuildReport, Documentation};
use az_plugin_host::runtime::server::RuntimeState;
use az_plugin_host::{configuration::DeliveryConfig, runtime::server::delivery::*};
use std::sync::atomic::{AtomicBool, AtomicUsize};
use tokio::sync::{Mutex, Notify};

struct Api {
    revision: Mutex<String>,
    marker: AtomicUsize,
    lists: AtomicUsize,
    commits: AtomicUsize,
    manifests: AtomicUsize,
    archived: AtomicBool,
    pause: AtomicBool,
    entered: Notify,
    release: Notify,
}
impl Default for Api {
    fn default() -> Self {
        Self {
            revision: Mutex::new("a".repeat(40)),
            marker: AtomicUsize::new(200),
            lists: AtomicUsize::new(0),
            commits: AtomicUsize::new(0),
            manifests: AtomicUsize::new(0),
            archived: AtomicBool::new(false),
            pause: AtomicBool::new(false),
            entered: Notify::new(),
            release: Notify::new(),
        }
    }
}
async fn respond(
    State(api): State<Arc<Api>>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    match uri.path() {
        "/users/example/repos" => {
            api.lists.fetch_add(1, Ordering::SeqCst);
            let archived = api.archived.load(Ordering::SeqCst);
            let etag = if archived { "archived" } else { "active" };
            if headers.get("if-none-match").is_some_and(|h| h == etag) {
                return StatusCode::NOT_MODIFIED.into_response();
            }
            let body = json!([{"clone_url":"https://127.0.0.1/example/widget.git", "full_name":"example/widget", "default_branch":"main", "archived":archived, "fork":false}]);
            ([("etag", etag)], axum::Json(body)).into_response()
        }
        "/repos/example/widget/commits" => {
            api.commits.fetch_add(1, Ordering::SeqCst);
            let sha = api.revision.lock().await.clone();
            if api.pause.load(Ordering::SeqCst) {
                api.entered.notify_one();
                api.release.notified().await;
            }
            axum::Json(json!([{"sha":sha}])).into_response()
        }
        "/repos/example/widget/contents/aio-delivery.toml" => {
            api.manifests.fetch_add(1, Ordering::SeqCst);
            if !uri
                .query()
                .is_some_and(|q| (q.starts_with("ref=") && q.len() == 44) || q == "ref=main")
            {
                return StatusCode::BAD_REQUEST.into_response();
            }
            match api.marker.load(Ordering::SeqCst) {
                404 => StatusCode::NOT_FOUND.into_response(),
                500 => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                _ => "version=1\n[build]\nenvironment='typescript'\ncommand=['fixture-build']\n"
                    .into_response(),
            }
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}
struct Server {
    api: Arc<Api>,
    base: String,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
}
impl Server {
    async fn start() -> Result<Self> {
        let api = Arc::new(Api::default());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}/", listener.local_addr()?);
        let routes = Router::new().fallback(get(respond)).with_state(api.clone());
        let task = tokio::spawn(async move { axum::serve(listener, routes).await });
        Ok(Self { api, base, task })
    }
    fn plugin(&self) -> DynPlugin<RuntimeAssembly> {
        Arc::new(GithubDeliveryDiscoveryPlugin {
            api_base: self.base.clone(),
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn configured(root: &std::path::Path) -> az_plugin_host::configuration::HostConfig {
    let mut config = super::super::fixture::config(root, "invalid database", "http://localhost");
    config.delivery = Some(DeliveryConfig {
        owner: "example".into(),
        discovery_interval_seconds: 1,
        revision_interval_seconds: 1,
    });
    config
}
const GIT: &str = "https://127.0.0.1/example/widget.git";
async fn source_when(
    catalog: &FileCatalog,
    predicate: impl Fn(&DeliverySource) -> bool,
) -> Result<DeliverySource> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(source) = catalog.get(GIT).await?
                && predicate(&source)
            {
                return Ok(source);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?
}
fn target(revision: char) -> Result<DeliveryTarget> {
    Ok(DeliveryTarget {
        branch: "main".into(),
        source_revision: revision.to_string().repeat(40),
        recipe: serde_json::from_value(
            json!({"environment":"typescript","command":["webhook-build"]}),
        )?,
    })
}

#[tokio::test]
#[ignore = "需要独立 AIO_DELIVERY_TOKEN；仅使用本机 HTTP 和文件存储"]
async fn discovery_survives_etags_rejects_stale_responses_and_unloads_without_stopping_delivery()
-> Result<()> {
    let server = Server::start().await?;
    let root = tempfile::tempdir()?;
    let path = root.path().join("catalog.json");
    let catalog = FileCatalog::open(&path);
    let mut selected = selection(catalog.clone(), true);
    selected.push(server.plugin());
    let mut host = Host::start_configured(configured(root.path()), selected.clone()).await?;
    assert!(host.state.service::<sqlx::PgPool>().is_err());
    let a = source_when(&catalog, |s| {
        s.enabled && s.source_revision == "a".repeat(40)
    })
    .await?;
    let first = claim(&host).await?;
    // 仓库分页返回 304 时，已登记来源仍须检查新提交。
    *server.api.revision.lock().await = "b".repeat(40);
    let b = source_when(&catalog, |s| s.source_revision == "b".repeat(40)).await?;
    assert!(b.generation > a.generation);
    assert!(!catalog.heartbeat(first.id, &first.lease).await?);
    assert!(server.api.lists.load(Ordering::SeqCst) > 1);
    let second = claim(&host).await?;
    assert_eq!(second.source_revision, "b".repeat(40));

    // API 暂时出错不能撤销有效来源，也不能将无效内容入队。
    server.api.marker.store(500, Ordering::SeqCst);
    let before = server.api.manifests.load(Ordering::SeqCst);
    *server.api.revision.lock().await = "c".repeat(40);
    tokio::time::timeout(Duration::from_secs(5), async {
        while server.api.manifests.load(Ordering::SeqCst) == before {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    assert_eq!(
        catalog.get(GIT).await?.context("来源缺失")?.generation,
        b.generation
    );
    server.api.marker.store(404, Ordering::SeqCst);
    let disabled = source_when(&catalog, |s| !s.enabled).await?;
    assert!(!catalog.heartbeat(second.id, &second.lease).await?);
    assert!(
        !catalog
            .update(GIT, Some(b.generation), Some(target('b')?))
            .await?
    );

    // 换发现插件或重启会重新扫描仓库；文件保存的墓碑和代数必须保留。
    host.reconfigure(selection(catalog.clone(), true)).await?;
    server.api.marker.store(200, Ordering::SeqCst);
    host.reconfigure(selected.clone()).await?;
    let c = source_when(&catalog, |s| {
        s.enabled && s.source_revision == "c".repeat(40)
    })
    .await?;
    assert!(c.generation > disabled.generation);

    // 较慢的 GitHub 响应不得覆盖另一个发现器已写入的目标。
    server.api.pause.store(true, Ordering::SeqCst);
    *server.api.revision.lock().await = "d".repeat(40);
    tokio::time::timeout(Duration::from_secs(5), server.api.entered.notified()).await?;
    assert!(
        catalog
            .update(GIT, Some(c.generation), Some(target('e')?))
            .await?
    );
    server.api.release.notify_one();
    // 等下一次请求进入暂停点，证明前一个陈旧响应已经被处理。
    tokio::time::timeout(Duration::from_secs(5), server.api.entered.notified()).await?;
    assert_eq!(
        catalog.get(GIT).await?.context("来源缺失")?.source_revision,
        "e".repeat(40)
    );

    // 停用必须取消网络等待，并排空本代任务；交付 HTTP 留在原监听器上。
    tokio::time::timeout(
        Duration::from_secs(2),
        host.reconfigure(selection(catalog.clone(), true)),
    )
    .await??;
    let requests = server.api.commits.load(Ordering::SeqCst);
    server.api.release.notify_waiters();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(server.api.commits.load(Ordering::SeqCst), requests);
    assert_eq!(
        catalog.get(GIT).await?.context("来源缺失")?.source_revision,
        "e".repeat(40)
    );
    host.stop().await?;
    drop(catalog);

    // 不装发现器即可恢复并完成它先前创建的构建。
    let catalog = FileCatalog::open(&path);
    let host =
        Host::start_configured(configured(root.path()), selection(catalog.clone(), true)).await?;
    let job = claim(&host).await?;
    assert_eq!(job.source_revision, "e".repeat(40));
    assert_eq!(job.recipe.command, vec!["webhook-build"]);
    let package = artifact(&job)?;
    upload(&host, &job, &package).await?.error_for_status()?;
    build_request(&host, &format!("/jobs/{}/complete", job.id))?
        .json(&BuildReport {
            lease: job.lease,
            error: None,
            documentation: Documentation::default(),
        })
        .send()
        .await?
        .error_for_status()?;
    assert_eq!(catalog.builds(GIT).await?[0]["state"], "active");
    assert_eq!(server.api.commits.load(Ordering::SeqCst), requests);
    host.stop().await
}

#[tokio::test]
async fn discovery_requires_one_source_provider_and_rebuilds_it_online() -> Result<()> {
    let server = Server::start().await?;
    let root = tempfile::tempdir()?;
    assert!(
        RuntimeState::initialize(configured(root.path()), vec![server.plugin()])
            .await
            .is_err()
    );
    let first = FileCatalog::open(&root.path().join("first.json"));
    let second = FileCatalog::open(&root.path().join("second.json"));
    struct Sources<const N: u8>(Arc<FileCatalog>);
    impl<const N: u8> Plugin<RuntimeAssembly> for Sources<N> {
        fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
            Box::pin(async move {
                assembly
                    .services
                    .add_builder(self.0.clone())
                    .bind::<dyn DeliverySources, FileCatalog>();
                Ok(())
            })
        }
    }
    assert!(
        RuntimeState::initialize(
            configured(root.path()),
            vec![
                server.plugin(),
                Arc::new(Sources::<1>(first.clone())),
                Arc::new(Sources::<2>(second.clone()))
            ]
        )
        .await
        .is_err()
    );
    let mut host = Host::start_configured(
        configured(root.path()),
        vec![server.plugin(), Arc::new(Sources::<1>(first.clone()))],
    )
    .await?;
    let old = source_when(&first, |s| s.enabled).await?;
    assert!(host.state.service::<dyn DeliveryService>().is_err());
    assert!(host.state.service::<sqlx::PgPool>().is_err());
    *server.api.revision.lock().await = "b".repeat(40);
    host.reconfigure(vec![
        server.plugin(),
        Arc::new(Sources::<2>(second.clone())),
    ])
    .await?;
    source_when(&second, |s| s.source_revision == "b".repeat(40)).await?;
    assert_eq!(
        first.get(GIT).await?.context("旧来源缺失")?.generation,
        old.generation
    );
    server.api.archived.store(true, Ordering::SeqCst);
    source_when(&second, |s| !s.enabled).await?;
    assert!(second.claim().await?.is_none());
    host.stop().await
}
