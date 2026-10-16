//! 应用路由和首页快照跟随当前代数，应用请求也必须参与插件资源排空。

use super::*;

async fn fixture(reject: bool, panic: bool) -> Result<Fixture> {
    let root = tempfile::tempdir()?;
    let trace = Arc::new(Trace::default());
    let plugins: Vec<DynPlugin<RuntimeAssembly>> = vec![
        Arc::new(ValuePlugin(Settings {
            value: 1,
            failure: Failure::None,
            trace: trace.clone(),
        })),
        Arc::new(Consumer),
    ];
    let host = RuntimeHost::initialize_with_router(
        config(root.path()),
        plugins.clone(),
        move |state, router| {
            let resource = state.service::<ValueResource>()?;
            if resource.value == 2 {
                assert!(!panic, "模拟应用路由 panic");
                ensure!(!reject, "模拟应用路由失败");
            }
            let application = Router::new()
                .route("/application", get(read))
                .route("/application/slow", get(slow))
                .fallback(get(read))
                .with_state(state.clone());
            Ok(router.merge(application))
        },
    )
    .await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let router = host.router();
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    Ok(Fixture {
        _root: root,
        host,
        plugins,
        trace,
        base,
        server,
        client: reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()?,
    })
}

#[tokio::test]
async fn application_routes_and_fallback_use_rebuilt_resources() -> Result<()> {
    let fixture = fixture(false, false).await?;
    let before = fixture.get("/application").await?;
    fixture
        .host
        .reconfigure(fixture.replacement(Failure::None))
        .await?;
    let after = fixture.get("/application").await?;
    assert_eq!(after["value"], 2);
    assert_ne!(after["id"], before["id"]);
    assert_eq!(fixture.get("/index.html").await?, after);
    fixture.host.shutdown().await?;
    assert_eq!(
        fixture
            .client
            .get(format!("{}/application", fixture.base))
            .send()
            .await?
            .status(),
        503
    );
    fixture.stop().await
}

#[tokio::test]
async fn application_router_errors_and_panics_restore_fresh_old_resources() -> Result<()> {
    for panic in [false, true] {
        let fixture = fixture(true, panic).await?;
        let before = fixture.get("/application").await?;
        assert!(
            fixture
                .host
                .reconfigure(fixture.replacement(Failure::None))
                .await
                .is_err()
        );
        let restored = fixture.get("/application").await?;
        assert_eq!(restored["value"], 1);
        assert_ne!(restored["id"], before["id"]);
        assert_eq!(fixture.get("/index.html").await?, restored);
        fixture.stop().await?;
    }
    Ok(())
}

#[tokio::test]
async fn application_requests_drain_before_resources_close() -> Result<()> {
    let fixture = fixture(false, false).await?;
    let client = fixture.client.clone();
    let url = format!("{}/application/slow", fixture.base);
    let request = tokio::spawn(async move { client.get(url).send().await });
    fixture.trace.started_request.notified().await;
    let host = fixture.host.clone();
    let change = tokio::spawn(async move { host.restart::<ValuePlugin>().await });
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!change.is_finished());
    assert_eq!(fixture.trace.stopped.load(Ordering::SeqCst), 0);
    fixture.trace.release_request.notify_one();
    // 处理必须完成，但已退休代数的响应流允许被关闭。
    let _response = request.await?;
    tokio::time::timeout(Duration::from_secs(5), change).await???;
    assert!(fixture.trace.request_finished.load(Ordering::SeqCst));
    assert_eq!(fixture.get("/application").await?["value"], 1);
    fixture.stop().await
}
