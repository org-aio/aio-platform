//! 通过持续监听的真实 HTTP 服务验证插件替换、资源重建与失败恢复。

use std::{
    any::{Any, TypeId},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use axum::{Json, Router, extract::State, routing::get};
use az_plugin_core::{DynPlugin, Plugin, PluginFuture};
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::{Resource, RuntimeAssembly, RuntimeHost, RuntimeState, tests::config};
use crate::runtime::server::http_error::RuntimeError;

#[derive(Default)]
struct Trace {
    next: AtomicUsize,
    opened: Mutex<Vec<Arc<ValueResource>>>,
    stable: Mutex<Vec<Arc<Stable>>>,
    stopped: AtomicUsize,
    stable_tasks_stopped: AtomicUsize,
    started_request: Notify,
    release_request: Notify,
    request_finished: AtomicBool,
    fail_recovery: AtomicBool,
}

trait ValueService: Any + Send + Sync {
    fn read(&self) -> Result<(usize, usize)>;
}
struct ValueResource {
    id: usize,
    value: usize,
    closed: AtomicBool,
}
impl ValueService for ValueResource {
    fn read(&self) -> Result<(usize, usize)> {
        ensure!(
            !self.closed.load(Ordering::SeqCst),
            "请求访问了已关闭的服务"
        );
        Ok((self.id, self.value))
    }
}

#[derive(Clone, Copy)]
enum Failure {
    None,
    Initialize,
    Panic,
    Stop,
    Routes,
}

#[derive(Clone)]
struct Settings {
    value: usize,
    failure: Failure,
    trace: Arc<Trace>,
}
struct ValuePlugin(Settings);
impl Plugin<RuntimeAssembly> for ValuePlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.services.add_value(self.0.clone());
            assembly.declare_resource::<ValueResource>()?;
            assembly.services.bind::<dyn ValueService, ValueResource>();
            assembly.on_initialize::<Self>(|state, _| {
                Box::pin(async move {
                    let settings = state.service::<Settings>()?;
                    let id = settings.trace.next.fetch_add(1, Ordering::SeqCst);
                    let resource = Arc::new(ValueResource {
                        id,
                        value: settings.value,
                        closed: AtomicBool::new(false),
                    });
                    settings
                        .trace
                        .opened
                        .lock()
                        .map_err(|_| anyhow::anyhow!("记录锁损坏"))?
                        .push(resource.clone());
                    state
                        .service::<Resource<ValueResource>>()?
                        .provide(resource)?;
                    ensure!(
                        !settings.trace.fail_recovery.load(Ordering::SeqCst),
                        "恢复故障"
                    );
                    match settings.failure {
                        Failure::Initialize => anyhow::bail!("初始化故障"),
                        Failure::Panic => panic!("初始化 panic"),
                        _ => Ok(()),
                    }
                })
            });
            assembly.on_stop::<Self>(|state, _| {
                Box::pin(async move {
                    let settings = state.service::<Settings>()?;
                    if let Some(resource) =
                        state.service::<Resource<ValueResource>>()?.initialized()
                    {
                        resource.closed.store(true, Ordering::SeqCst);
                        settings.trace.stopped.fetch_add(1, Ordering::SeqCst);
                    }
                    ensure!(!matches!(settings.failure, Failure::Stop), "停止故障");
                    Ok(())
                })
            });
            if matches!(self.0.failure, Failure::Routes) {
                assembly.add_routes(|_| panic!("路由装配故障"));
            }
            Ok(())
        })
    }
}

#[dill::component]
#[dill::scope(dill::Singleton)]
struct Cached {
    value: Arc<dyn ValueService>,
}
struct Consumer;
impl Plugin<RuntimeAssembly> for Consumer {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            // 只有 Dill 注入依赖，没有显式 Plugin::dependencies，也必须正确重建。
            assembly.services.add::<Cached>();
            assembly.require_service::<Cached>();
            Ok(())
        })
    }
}
struct Routes;
impl Plugin<RuntimeAssembly> for Routes {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.require_service::<Cached>();
            assembly.require_service::<Settings>();
            assembly.add_routes(|_| {
                Router::new()
                    .route("/value", get(read))
                    .route("/slow", get(slow))
            });
            Ok(())
        })
    }
}
async fn read(State(state): State<RuntimeState>) -> Result<Json<Value>, RuntimeError> {
    let (id, value) = state.service::<Cached>()?.value.read()?;
    Ok(Json(json!({"id":id,"value":value})))
}
async fn slow(State(state): State<RuntimeState>) -> Result<Json<Value>, RuntimeError> {
    let settings = state.service::<Settings>()?;
    let cached = state.service::<Cached>()?;
    let before = cached.value.read()?;
    settings.trace.started_request.notify_one();
    settings.trace.release_request.notified().await;
    assert_eq!(before, state.service::<Cached>()?.value.read()?);
    settings
        .trace
        .request_finished
        .store(true, Ordering::SeqCst);
    Ok(Json(json!({"value":before.1})))
}

struct Stable {
    id: usize,
    trace: Arc<Trace>,
}
struct StablePlugin(Arc<Trace>);
impl Plugin<RuntimeAssembly> for StablePlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.services.add_value(Stable {
                id: self.0.next.fetch_add(1, Ordering::SeqCst),
                trace: self.0.clone(),
            });
            assembly.on_ready::<Self>(|state, tasks| {
                Box::pin(async move {
                    let stable = state.service::<Stable>()?;
                    stable
                        .trace
                        .stable
                        .lock()
                        .map_err(|_| anyhow::anyhow!("记录锁损坏"))?
                        .push(stable.clone());
                    let cancellation = tasks.cancellation();
                    tasks.spawn(async move {
                        cancellation.cancelled().await;
                        stable
                            .trace
                            .stable_tasks_stopped
                            .fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    })
                })
            });
            assembly.add_routes(|_| {
                Router::new().route(
                    "/stable",
                    get(|State(state): State<RuntimeState>| async move {
                        Ok::<_, RuntimeError>(Json(json!({"id":state.service::<Stable>()?.id})))
                    }),
                )
            });
            Ok(())
        })
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    host: RuntimeHost,
    plugins: Vec<DynPlugin<RuntimeAssembly>>,
    trace: Arc<Trace>,
    client: reqwest::Client,
    base: String,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
}
impl Fixture {
    async fn new() -> Result<Self> {
        let root = tempfile::tempdir()?;
        let trace = Arc::new(Trace::default());
        let plugins: Vec<DynPlugin<RuntimeAssembly>> = vec![
            Arc::new(ValuePlugin(Settings {
                value: 1,
                failure: Failure::None,
                trace: trace.clone(),
            })),
            Arc::new(Consumer),
            Arc::new(Routes),
            Arc::new(StablePlugin(trace.clone())),
            Arc::new(ObserverPlugin),
        ];
        let host = RuntimeHost::initialize(config(root.path()), plugins.clone()).await?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let router = host.router();
        let server = tokio::spawn(async move { axum::serve(listener, router).await });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()?;
        Ok(Self {
            _root: root,
            host,
            plugins,
            trace,
            client,
            base,
            server,
        })
    }
    fn replacement(&self, failure: Failure) -> Vec<DynPlugin<RuntimeAssembly>> {
        let mut plugins = self.plugins.clone();
        plugins[0] = Arc::new(ValuePlugin(Settings {
            value: 2,
            failure,
            trace: self.trace.clone(),
        }));
        plugins
    }
    async fn get(&self, path: &str) -> Result<Value> {
        Ok(self
            .client
            .get(format!("{}{path}", self.base))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
    async fn stop(&self) -> Result<()> {
        let result = self.host.shutdown().await;
        self.server.abort();
        result
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

#[tokio::test]
async fn replaces_and_restarts_injected_services_preserving_unrelated_singletons_and_tasks()
-> Result<()> {
    let fixture = Fixture::new().await?;
    let before = fixture.get("/value").await?;
    let stable = fixture.get("/stable").await?;
    fixture
        .host
        .reconfigure(fixture.replacement(Failure::None))
        .await?;
    let after = fixture.get("/value").await?;
    assert_eq!(after["value"], 2);
    assert_eq!(fixture.get("/observer").await?, after);
    assert_ne!(before["id"], after["id"]);
    assert_eq!(fixture.get("/stable").await?, stable);
    assert_eq!(
        fixture
            .trace
            .stable
            .lock()
            .map_err(|_| anyhow::anyhow!("记录锁损坏"))?
            .len(),
        1
    );
    assert_eq!(fixture.trace.stable_tasks_stopped.load(Ordering::SeqCst), 0);
    fixture.host.restart::<ValuePlugin>().await?;
    let restarted = fixture.get("/value").await?;
    assert_eq!(restarted["value"], 2);
    assert_ne!(after["id"], restarted["id"]);
    assert_eq!(fixture.get("/stable").await?, stable);
    fixture.stop().await?;
    assert_eq!(fixture.trace.stable_tasks_stopped.load(Ordering::SeqCst), 1);
    let opened = fixture
        .trace
        .opened
        .lock()
        .map_err(|_| anyhow::anyhow!("记录锁损坏"))?;
    assert_eq!(opened.len(), 3);
    assert!(
        opened
            .iter()
            .all(|resource| resource.closed.load(Ordering::SeqCst))
    );
    Ok(())
}

#[tokio::test]
async fn failed_initialization_panic_and_route_factory_restore_fresh_previous_resources()
-> Result<()> {
    for failure in [Failure::Initialize, Failure::Panic, Failure::Routes] {
        let fixture = Fixture::new().await?;
        let before = fixture.get("/value").await?;
        let error = fixture
            .host
            .reconfigure(fixture.replacement(failure))
            .await
            .err()
            .context("候选必须失败")?;
        assert!(format!("{error:#}").contains("恢复服务"), "{error:#}");
        let restored = fixture.get("/value").await?;
        assert_eq!(restored["value"], 1);
        assert_ne!(restored["id"], before["id"]);
        assert_eq!(fixture.trace.stopped.load(Ordering::SeqCst), 2);
        assert_eq!(fixture.trace.stable_tasks_stopped.load(Ordering::SeqCst), 0);
        fixture.stop().await?;
        assert_eq!(fixture.trace.stopped.load(Ordering::SeqCst), 3);
    }
    Ok(())
}

#[tokio::test]
async fn rejects_missing_services_before_stopping_and_unloads_routes_online() -> Result<()> {
    let fixture = Fixture::new().await?;
    let before = fixture.get("/value").await?;
    let mut missing = fixture.plugins.clone();
    missing.remove(0);
    assert!(fixture.host.reconfigure(missing).await.is_err());
    assert_eq!(fixture.get("/value").await?, before);
    assert_eq!(fixture.trace.stopped.load(Ordering::SeqCst), 0);
    fixture
        .host
        .reconfigure(vec![fixture.plugins[3].clone()])
        .await?;
    assert_eq!(
        fixture
            .client
            .get(format!("{}/value", fixture.base))
            .send()
            .await?
            .status(),
        404
    );
    assert_eq!(fixture.trace.stopped.load(Ordering::SeqCst), 1);
    fixture.get("/stable").await?;
    fixture.host.reconfigure(fixture.plugins.clone()).await?;
    assert_eq!(fixture.get("/value").await?["value"], 1);
    fixture.stop().await
}

#[tokio::test]
async fn caller_cancellation_does_not_interrupt_request_drain_or_commit() -> Result<()> {
    let fixture = Fixture::new().await?;
    let request = fixture.client.get(format!("{}/slow", fixture.base)).send();
    let request = tokio::spawn(request);
    tokio::time::timeout(
        Duration::from_secs(3),
        fixture.trace.started_request.notified(),
    )
    .await?;
    let host = fixture.host.clone();
    let plugins = fixture.replacement(Failure::None);
    let change = tokio::spawn(async move { host.reconfigure(plugins).await });
    // 503 证明已进入排空阶段，旧请求完成前资源仍保持打开。
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if fixture
                .client
                .get(format!("{}/value", fixture.base))
                .send()
                .await?
                .status()
                == 503
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    assert_eq!(fixture.trace.stopped.load(Ordering::SeqCst), 0);
    change.abort();
    fixture.trace.release_request.notify_one();
    // 旧代数会终止响应流；已经进入处理的工作必须完成后才释放资源。
    let _response = request.await?;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let response = fixture
                .client
                .get(format!("{}/value", fixture.base))
                .send()
                .await?;
            if response.status().is_success() {
                assert_eq!(response.json::<Value>().await?["value"], 2);
                break;
            }
            tokio::task::yield_now().await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    assert!(fixture.trace.request_finished.load(Ordering::SeqCst));
    assert_eq!(fixture.trace.stable_tasks_stopped.load(Ordering::SeqCst), 0);
    fixture.stop().await
}

#[tokio::test]
async fn failed_recovery_stops_retained_tasks_and_rejects_future_calls() -> Result<()> {
    let fixture = Fixture::new().await?;
    fixture.trace.fail_recovery.store(true, Ordering::SeqCst);
    let error = fixture
        .host
        .reconfigure(fixture.replacement(Failure::None))
        .await
        .err()
        .context("恢复必须失败")?;
    assert!(format!("{error:#}").contains("恢复也失败"));
    assert_eq!(
        fixture
            .client
            .get(format!("{}/stable", fixture.base))
            .send()
            .await?
            .status(),
        503
    );
    assert_eq!(fixture.trace.stable_tasks_stopped.load(Ordering::SeqCst), 1);
    assert!(fixture.host.restart::<ValuePlugin>().await.is_err());
    fixture.stop().await
}

#[tokio::test]
async fn stop_failure_fails_closed_without_starting_overlapping_resources() -> Result<()> {
    let fixture = Fixture::new().await?;
    fixture
        .host
        .reconfigure(fixture.replacement(Failure::Stop))
        .await?;
    assert!(fixture.host.restart::<ValuePlugin>().await.is_err());
    assert_eq!(
        fixture
            .trace
            .opened
            .lock()
            .map_err(|_| anyhow::anyhow!("记录锁损坏"))?
            .len(),
        2
    );
    assert_eq!(
        fixture
            .client
            .get(format!("{}/value", fixture.base))
            .send()
            .await?
            .status(),
        503
    );
    assert_eq!(fixture.trace.stable_tasks_stopped.load(Ordering::SeqCst), 1);
    assert!(fixture.stop().await.is_err());
    Ok(())
}

struct Cyclic;
impl Plugin<RuntimeAssembly> for Cyclic {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![TypeId::of::<Self>()]
    }
    fn build<'a>(&'a self, _: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async { panic!("不应构建循环候选") })
    }
}

#[tokio::test]
async fn rejects_duplicate_plugins_and_cycles_without_touching_active_generation() -> Result<()> {
    let fixture = Fixture::new().await?;
    let before = fixture.get("/value").await?;
    for plugin in [
        fixture.plugins[0].clone(),
        Arc::new(Cyclic),
        Arc::new(ForeignBinding),
        Arc::new(DuplicateService),
        Arc::new(PanickingBuild),
    ] {
        let mut invalid = fixture.plugins.clone();
        invalid.push(plugin);
        assert!(fixture.host.reconfigure(invalid).await.is_err());
        assert_eq!(fixture.get("/value").await?, before);
    }
    assert_eq!(fixture.trace.stopped.load(Ordering::SeqCst), 0);
    fixture.stop().await
}

#[dill::component]
#[dill::scope(dill::Singleton)]
struct Observer {
    catalog: dill::CatalogWeakRef,
}
struct ObserverPlugin;
impl Plugin<RuntimeAssembly> for ObserverPlugin {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.services.add::<Observer>();
            assembly.require_service::<Observer>();
            assembly.add_routes(|_| {
                Router::new().route(
                    "/observer",
                    get(|State(state): State<RuntimeState>| async move {
                        let observer = state.service::<Observer>()?;
                        let catalog = observer.catalog.upgrade();
                        let (id, value) = catalog.get_one::<Cached>()?.value.read()?;
                        Ok::<_, RuntimeError>(Json(json!({"id":id,"value":value})))
                    }),
                )
            });
            Ok(())
        })
    }
}

struct ForeignBinding;
impl Plugin<RuntimeAssembly> for ForeignBinding {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.services.bind::<dyn ValueService, ValueResource>();
            Ok(())
        })
    }
}
struct DuplicateService;
impl Plugin<RuntimeAssembly> for DuplicateService {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.services.add_value(ValueResource {
                id: 100,
                value: 100,
                closed: AtomicBool::new(false),
            });
            Ok(())
        })
    }
}
struct PanickingBuild;
impl Plugin<RuntimeAssembly> for PanickingBuild {
    fn build<'a>(&'a self, _: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        panic!("同步装配方法 panic")
    }
}

#[tokio::test]
async fn concurrent_restarts_use_latest_generation_and_stop_each_task_once() -> Result<()> {
    let fixture = Fixture::new().await?;
    let (value, stable) = tokio::join!(
        fixture.host.restart::<ValuePlugin>(),
        fixture.host.restart::<StablePlugin>()
    );
    value?;
    stable?;
    assert_eq!(fixture.get("/value").await?["value"], 1);
    fixture.get("/stable").await?;
    assert_eq!(fixture.trace.stopped.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.trace.stable_tasks_stopped.load(Ordering::SeqCst), 1);
    fixture.stop().await?;
    assert_eq!(fixture.trace.stopped.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.trace.stable_tasks_stopped.load(Ordering::SeqCst), 2);
    Ok(())
}
