use super::*;

#[path = "../support/remote.rs"]
mod upstream;
use upstream::Upstream;

async fn write_imports(path: &std::path::Path, imports: Vec<Value>) -> Result<()> {
    let pending = path.with_extension("pending");
    tokio::fs::write(
        &pending,
        serde_json::to_vec(&json!({"schema_version":1,"imports":imports}))?,
    )
    .await?;
    tokio::fs::rename(pending, path).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "需要 AIO_REMOTE_TEST_CREDENTIAL=remote-test；无需数据库或解释器"]
async fn native_mcp_and_grpc_calls_share_contracts_deadlines_and_pinned_reload_snapshots()
-> Result<()> {
    anyhow::ensure!(
        std::env::var("AIO_REMOTE_TEST_CREDENTIAL")? == "remote-test",
        "只使用测试凭据"
    );
    let first = Upstream::open().await?;
    let second = Upstream::open().await?;
    let root = tempfile::tempdir()?;
    let path = root.path().join("remote.json");
    write_imports(&path, first.imports()).await?;
    let mut settings = config(root.path());
    settings.remote_capabilities = Some(path.clone());
    let state = RuntimeState::initialize(
        settings,
        identity_support::with_identity(
            Arc::new(Identity::default()),
            vec![
                Arc::new(CapabilityServicePlugin),
                Arc::new(RemoteCapabilitiesPlugin),
            ],
        ),
    )
    .await?;
    assert!(state.service::<ScriptRuntime>().is_err());
    #[cfg(feature = "postgres")]
    assert!(state.service::<sqlx::PgPool>().is_err());
    assert!(!root.path().join("cache").exists());
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        exercise(&state, &path, &first, &second),
    )
    .await;
    let cleanup = state.shutdown().await;
    result??;
    cleanup?;
    Ok(())
}

async fn exercise(
    state: &RuntimeState,
    path: &std::path::Path,
    first: &Upstream,
    second: &Upstream,
) -> Result<()> {
    let mut caller = scope();
    caller.grants.runtime = vec![
        "/v1/remote/mcp".into(),
        "/v1/remote/grpc".into(),
        "/v1/remote/added".into(),
    ];
    let pinned = state.capability_invocation(&caller, None)?;
    for protocol in ["mcp", "grpc"] {
        let target = format!("/v1/remote/{protocol}");
        let before = first.counts.calls.load(Ordering::SeqCst);
        assert_eq!(
            pinned.invoke(&target, &json!({"value":21})).await?,
            json!({"value":42})
        );
        assert!(
            pinned
                .invoke(&target, &json!({"value":"wrong"}))
                .await
                .is_err()
        );
        assert_eq!(first.counts.calls.load(Ordering::SeqCst), before + 1);
        assert!(pinned.invoke(&target, &json!({"value":-2})).await.is_err());
        assert_eq!(
            first.counts.calls.load(Ordering::SeqCst),
            before + 2,
            "外部副作用不能重试"
        );
        let mut other = caller.clone();
        other.context.tenant_id = Some("other".into());
        assert!(
            state
                .capability_invocation(&other, None)?
                .invoke(&target, &json!({"value":1}))
                .await
                .is_err()
        );
        other = caller.clone();
        other.grants.runtime.clear();
        assert!(
            state
                .capability_invocation(&other, None)?
                .invoke(&target, &json!({"value":1}))
                .await
                .is_err()
        );
        assert_eq!(first.counts.calls.load(Ordering::SeqCst), before + 2);

        let invocation = state
            .capability_invocation(&caller, Some(Instant::now() + Duration::from_millis(500)))?;
        let started = Instant::now();
        assert!(
            invocation
                .invoke(&target, &json!({"value":-1}))
                .await
                .is_err()
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        tokio::time::timeout(Duration::from_secs(3), async {
            while first.counts.active.load(Ordering::SeqCst) != 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        assert_eq!(first.counts.calls.load(Ordering::SeqCst), before + 3);
    }
    assert!(first.counts.credentials.load(Ordering::SeqCst) > 0);
    assert!(first.counts.deadlines.load(Ordering::SeqCst) > 0);
    assert!(first.counts.last_timeout_micros.load(Ordering::SeqCst) <= 500_000);
    for input in [-3, -4] {
        assert!(
            pinned
                .invoke("/v1/remote/mcp", &json!({"value":input}))
                .await
                .is_err()
        );
    }

    let mut imports = second.imports();
    let mut added = imports[0].clone();
    added["capability"]["path"] = json!("/v1/remote/added");
    imports.push(added);
    write_imports(path, imports).await?;
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if state
                .service::<RuntimeCapabilities>()?
                .catalog("test")?
                .len()
                == 3
            {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await??;
    let current = state.capability_invocation(&caller, None)?;
    assert!(!pinned.available("/v1/remote/added"));
    assert!(current.available("/v1/remote/added"));
    for protocol in ["mcp", "grpc"] {
        let target = format!("/v1/remote/{protocol}");
        let before = first.counts.calls.load(Ordering::SeqCst);
        let next_before = second.counts.calls.load(Ordering::SeqCst);
        assert_eq!(
            pinned.invoke(&target, &json!({"value":7})).await?,
            json!({"value":14})
        );
        assert_eq!(first.counts.calls.load(Ordering::SeqCst), before + 1);
        assert_eq!(second.counts.calls.load(Ordering::SeqCst), next_before);
        assert_eq!(
            current.invoke(&target, &json!({"value":9})).await?,
            json!({"value":18})
        );
        assert_eq!(second.counts.calls.load(Ordering::SeqCst), next_before + 1);
    }
    state.shutdown().await?;
    assert!(
        pinned
            .invoke("/v1/remote/mcp", &json!({"value":1}))
            .await
            .is_err()
    );
    assert!(
        current
            .invoke("/v1/remote/grpc", &json!({"value":1}))
            .await
            .is_err()
    );
    Ok(())
}
