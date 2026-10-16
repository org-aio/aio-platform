use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result};
use axum::{
    body::{Body, Bytes},
    extract::State,
    routing::get,
};
use az_plugin_core::{Plugin, PluginFuture};
use futures_util::StreamExt;
use tokio::{net::TcpListener, sync::Notify};

use super::*;

#[derive(Clone, Copy, Default, PartialEq)]
enum Mode {
    #[default]
    Normal,
    StartFailure,
    ReadyFailure,
    WaitForCancellation,
    StopFailure,
    StopPanic,
    WaitOnStop,
}

#[derive(Clone, Default)]
struct Trace {
    mode: Mode,
    events: Arc<Mutex<Vec<&'static str>>>,
    resource: Arc<Mutex<Option<TcpListener>>>,
    address: Arc<Mutex<Option<SocketAddr>>>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
    stopped: Arc<Notify>,
}

impl Trace {
    fn record(&self, event: &'static str) -> Result<()> {
        self.events
            .lock()
            .map_err(|_| anyhow::anyhow!("测试锁损坏"))?
            .push(event);
        Ok(())
    }
    fn events(&self) -> Result<Vec<&'static str>> {
        Ok(self
            .events
            .lock()
            .map_err(|_| anyhow::anyhow!("测试锁损坏"))?
            .clone())
    }
    async fn resource_released(&self) -> Result<()> {
        let address = *self
            .address
            .lock()
            .map_err(|_| anyhow::anyhow!("测试锁损坏"))?;
        let _listener = TcpListener::bind(address.context("未绑定测试端口")?).await?;
        Ok(())
    }
}

struct Foundation(Trace);
impl Plugin<RuntimeAssembly> for Foundation {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.services.add_value(self.0.clone());
            assembly.on_start::<Self>(|state, _| {
                Box::pin(async move { state.service::<Trace>()?.record("foundation.start") })
            });
            assembly.on_ready::<Self>(|state, _| {
                Box::pin(async move { state.service::<Trace>()?.record("foundation.ready") })
            });
            assembly.on_stop::<Self>(|state, _| {
                Box::pin(async move {
                    let trace = state.service::<Trace>()?;
                    trace.record("foundation.stop")?;
                    trace.stopped.notify_one();
                    Ok(())
                })
            });
            Ok(())
        })
    }
}

struct Dependent;
impl Plugin<RuntimeAssembly> for Dependent {
    fn dependencies(&self) -> Vec<TypeId> {
        vec![TypeId::of::<Foundation>()]
    }
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly.on_start::<Self>(|state, tasks| {
                Box::pin(async move {
                    let trace = state.service::<Trace>()?;
                    trace.record("dependent.start")?;
                    let listener = TcpListener::bind("127.0.0.1:0").await?;
                    *trace
                        .address
                        .lock()
                        .map_err(|_| anyhow::anyhow!("测试锁损坏"))? = Some(listener.local_addr()?);
                    *trace
                        .resource
                        .lock()
                        .map_err(|_| anyhow::anyhow!("测试锁损坏"))? = Some(listener);
                    trace.entered.notify_one();
                    if trace.mode == Mode::StartFailure {
                        anyhow::bail!("start failure");
                    }
                    if trace.mode == Mode::WaitForCancellation {
                        tasks.cancellation().cancelled().await;
                    }
                    Ok(())
                })
            });
            assembly.on_ready::<Self>(|state, tasks| {
                Box::pin(async move {
                    let trace = state.service::<Trace>()?;
                    trace.record("dependent.ready")?;
                    let cancellation = tasks.cancellation();
                    let worker = trace.clone();
                    tasks.spawn(async move {
                        cancellation.cancelled().await;
                        worker.record("task.finished")
                    })?;
                    if trace.mode == Mode::ReadyFailure {
                        anyhow::bail!("ready failure");
                    }
                    Ok(())
                })
            });
            assembly.on_stop::<Self>(|state, _| {
                Box::pin(async move {
                    let trace = state.service::<Trace>()?;
                    trace.record("dependent.stop")?;
                    trace
                        .resource
                        .lock()
                        .map_err(|_| anyhow::anyhow!("测试锁损坏"))?
                        .take();
                    match trace.mode {
                        Mode::StopFailure => anyhow::bail!("stop failure"),
                        Mode::StopPanic => panic!("stop panic"),
                        Mode::WaitOnStop => {
                            trace.entered.notify_one();
                            trace.release.notified().await;
                        }
                        _ => {}
                    }
                    Ok(())
                })
            });
            assembly.add_routes(|_| {
                Router::new()
                    .route("/probe", get(|| async { "ok" }))
                    .route("/write", get(write_request))
                    .route(
                        "/stream",
                        get(|| async {
                            let stream = futures_util::stream::once(async {
                                Ok::<_, std::io::Error>(Bytes::from_static(b"ready"))
                            })
                            .chain(futures_util::stream::pending());
                            Body::from_stream(stream)
                        }),
                    )
            });
            Ok(())
        })
    }
}

async fn write_request(
    State(state): State<RuntimeState>,
) -> Result<&'static str, super::super::http_error::RuntimeError> {
    let trace = state.service::<Trace>()?;
    trace.entered.notify_one();
    trace.release.notified().await;
    trace.record("request.committed")?;
    Ok("committed")
}

async fn runtime(mode: Mode) -> Result<(tempfile::TempDir, RuntimeState, Trace)> {
    let root = tempfile::tempdir()?;
    let trace = Trace {
        mode,
        ..Trace::default()
    };
    // 故意按反序提交插件，验证停止顺序来自内核已排序的构建过程。
    let state = RuntimeState::assemble(
        tests::config(root.path()),
        vec![Arc::new(Dependent), Arc::new(Foundation(trace.clone()))],
    )
    .await?;
    Ok((root, state, trace))
}

async fn signalled(notify: &Notify) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(5), notify.notified()).await?;
    Ok(())
}

#[tokio::test]
async fn partial_start_is_compensated_before_returning_error() -> Result<()> {
    let (_root, state, trace) = runtime(Mode::StartFailure).await?;
    let error = state
        .extensions
        .start(&state, true)
        .await
        .err()
        .context("必须启动失败")?;
    assert!(format!("{error:#}").contains("start failure"));
    assert_eq!(
        trace.events()?,
        [
            "foundation.start",
            "dependent.start",
            "dependent.stop",
            "foundation.stop"
        ]
    );
    trace.resource_released().await?;
    state.shutdown().await?;
    assert_eq!(trace.events()?.len(), 4);
    Ok(())
}

#[tokio::test]
async fn ready_failure_drains_tasks_before_releasing_resources() -> Result<()> {
    let (_root, state, trace) = runtime(Mode::ReadyFailure).await?;
    assert!(state.extensions.start(&state, true).await.is_err());
    assert_eq!(
        trace.events()?,
        [
            "foundation.start",
            "dependent.start",
            "foundation.ready",
            "dependent.ready",
            "task.finished",
            "dependent.stop",
            "foundation.stop"
        ]
    );
    trace.resource_released().await
}

#[tokio::test]
async fn dropping_initialization_cancels_and_cleans_partial_resources() -> Result<()> {
    let (_root, state, trace) = runtime(Mode::WaitForCancellation).await?;
    let owned = state.clone();
    let startup = tokio::spawn(async move { owned.extensions.start(&owned, true).await });
    signalled(&trace.entered).await?;
    startup.abort();
    assert!(startup.await.is_err());
    signalled(&trace.stopped).await?;
    assert_eq!(
        trace.events()?,
        [
            "foundation.start",
            "dependent.start",
            "dependent.stop",
            "foundation.stop"
        ]
    );
    trace.resource_released().await
}

#[tokio::test]
async fn cleanup_continues_after_error_or_panic_and_is_idempotent() -> Result<()> {
    for mode in [Mode::StopFailure, Mode::StopPanic] {
        let (_root, state, trace) = runtime(mode).await?;
        state.extensions.start(&state, true).await?;
        let (first, second) = tokio::join!(state.shutdown(), state.shutdown());
        let first = first.err().context("停止错误不能被忽略")?;
        let second = second.err().context("再次停止应返回同一错误")?;
        assert_eq!(format!("{first:#}"), format!("{second:#}"));
        assert_eq!(
            trace
                .events()?
                .iter()
                .filter(|event| **event == "dependent.stop")
                .count(),
            1
        );
        assert_eq!(trace.events()?.last(), Some(&"foundation.stop"));
        assert!(
            state
                .plugin_tasks::<Dependent>()?
                .spawn(async { Ok(()) })
                .is_err()
        );
        trace.resource_released().await?;
    }
    Ok(())
}

#[tokio::test]
async fn dropping_shutdown_waiter_does_not_cancel_cleanup() -> Result<()> {
    let (_root, state, trace) = runtime(Mode::WaitOnStop).await?;
    state.extensions.start(&state, true).await?;
    signalled(&trace.entered).await?;
    let owned = state.clone();
    let shutdown = tokio::spawn(async move { owned.shutdown().await });
    signalled(&trace.entered).await?;
    shutdown.abort();
    assert!(shutdown.await.is_err());
    trace.release.notify_one();
    signalled(&trace.stopped).await?;
    state.shutdown().await?;
    trace.resource_released().await
}

#[tokio::test]
async fn shutdown_waits_for_finite_work_and_reports_task_failures() -> Result<()> {
    let (_root, state, trace) = runtime(Mode::Normal).await?;
    state.extensions.start(&state, true).await?;
    signalled(&trace.entered).await?;
    let worker = trace.clone();
    let cancellation = state.plugin_tasks::<Dependent>()?.cancellation();
    state.plugin_tasks::<Dependent>()?.spawn(async move {
        cancellation.cancelled().await;
        worker.entered.notify_one();
        worker.release.notified().await;
        worker.record("transaction.committed")?;
        anyhow::bail!("task failure after commit");
    })?;
    let owned = state.clone();
    let shutdown = tokio::spawn(async move { owned.shutdown().await });
    signalled(&trace.entered).await?;
    assert!(!shutdown.is_finished());
    assert!(!trace.events()?.contains(&"dependent.stop"));
    trace.release.notify_one();
    let result = shutdown.await?.err().context("任务失败需要报告")?;
    assert!(format!("{result:#}").contains("task failure after commit"));
    let events = trace.events()?;
    assert!(
        events.iter().position(|e| *e == "transaction.committed")
            < events.iter().position(|e| *e == "dependent.stop")
    );
    trace.resource_released().await
}

#[tokio::test]
async fn shutdown_rejects_new_http_requests_and_drains_handlers_and_streams() -> Result<()> {
    let (_root, state, trace) = runtime(Mode::Normal).await?;
    state.extensions.start(&state, true).await?;
    signalled(&trace.entered).await?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let router = super::super::router(state.clone());
    let mut servers = tokio::task::JoinSet::new();
    servers.spawn(async move { axum::serve(listener, router).await });
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()?;
    let mut stream = client.get(format!("{origin}/stream")).send().await?;
    assert_eq!(stream.chunk().await?.as_deref(), Some(b"ready".as_slice()));
    let write = client.get(format!("{origin}/write")).send();
    let write = tokio::spawn(write);
    signalled(&trace.entered).await?;
    let owned = state.clone();
    let shutdown = tokio::spawn(async move { owned.shutdown().await });
    state.extensions.lifecycle.cancellation.cancelled().await;
    assert_eq!(
        client.get(format!("{origin}/probe")).send().await?.status(),
        503
    );
    assert!(!shutdown.is_finished());
    trace.release.notify_one();
    // 取消响应可能使传输返回错误，但已进入的变更必须完成。
    let _ = write.await?;
    let _ = stream.chunk().await;
    tokio::time::timeout(Duration::from_secs(5), shutdown).await???;
    assert!(trace.events()?.contains(&"request.committed"));
    trace.resource_released().await
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn shutdown_leaves_unclaimed_publications_durable() -> Result<()> {
    let database = std::env::var("AIO_ENHANCEMENT_TEST_DATABASE_URL")?;
    anyhow::ensure!(
        database.contains("127.0.0.1") && database.contains("_test"),
        "只接受本机独立测试库"
    );
    let root = tempfile::tempdir()?;
    let state = RuntimeState::isolated_admin_test(
        Arc::new(tests::NoIdentity),
        &database,
        "http://127.0.0.1",
        root.path(),
    )
    .await?;
    let tenant = uuid::Uuid::new_v4().to_string();
    let job = state
        .store()?
        .queue_publish_job(
            &tenant,
            "fixture",
            &format!("https://example.invalid/{tenant}.git"),
            &"a".repeat(64),
            crate::runtime::PluginRuntime::PageDefinition,
            0,
        )
        .await?;
    let _slots = state
        .publication_slots
        .clone()
        .acquire_many_owned(2)
        .await?;
    state.start_publish_job(job.clone())?;
    tokio::time::timeout(Duration::from_secs(5), state.shutdown()).await??;
    assert!(state.store()?.pool.is_closed());
    let pool = sqlx::PgPool::connect(&database).await?;
    let status: String = sqlx::query_scalar("SELECT state FROM plugin_publish_jobs WHERE id=$1")
        .bind(&job.id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "queued");
    sqlx::query("DELETE FROM plugin_publish_jobs WHERE id=$1")
        .bind(&job.id)
        .execute(&pool)
        .await?;
    pool.close().await;
    Ok(())
}
