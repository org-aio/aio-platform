#![cfg(feature = "server")]

#[path = "support/identity.rs"]
mod identity_support;
#[cfg(all(feature = "remote-mcp", feature = "remote-grpc"))]
#[path = "capabilities/remote.rs"]
mod remote;
#[path = "support/capabilities.rs"]
mod support;

use std::{
    sync::{Arc, Mutex, atomic::Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use az_plugin_core::DynPlugin;
use az_plugin_host::runtime::server::{RuntimeState, plugins::*};
use serde_json::{Value, json};
use tower::ServiceExt;

use support::*;

fn plugins(identity: Arc<Identity>, probe: Arc<Probe>) -> Vec<DynPlugin<RuntimeAssembly>> {
    identity_support::with_identity(
        identity,
        vec![
            Arc::new(CapabilityServicePlugin),
            Arc::new(CapabilitiesPlugin),
            Arc::new(ProviderPlugin(probe, 42)),
        ],
    )
}

#[tokio::test]
async fn native_invocation_and_http_directory_work_without_script_or_database() -> Result<()> {
    let root = tempfile::tempdir()?;
    let identity = Arc::new(Identity::default());
    let probe = Arc::new(Probe::default());
    let state = RuntimeState::initialize(
        config(root.path()),
        plugins(identity.clone(), probe.clone()),
    )
    .await?;
    assert!(state.service::<ScriptRuntime>().is_err());
    #[cfg(feature = "postgres")]
    assert!(state.service::<sqlx::PgPool>().is_err());
    assert!(state.service::<az_plugin_runtime::Keyring>().is_err());
    assert!(!root.path().join("cache").exists());
    let invocation = state.capability_invocation(&scope(), None)?;
    assert!(invocation.available(PATH));
    assert_eq!(invocation.invoke(PATH, &json!(21)).await?, 42);
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);

    let router = az_plugin_host::runtime::server::router(state.clone())?;
    for (actor, status, count) in [
        (None, StatusCode::UNAUTHORIZED, None),
        (Some("limited"), StatusCode::FORBIDDEN, None),
        (Some("manager"), StatusCode::OK, Some(1)),
        (Some("other"), StatusCode::OK, Some(0)),
    ] {
        let mut request = Request::builder().uri("/api/runtime/capabilities");
        if let Some(actor) = actor {
            request = request.header("authorization", format!("Bearer {actor}"));
        }
        let response = router.clone().oneshot(request.body(Body::empty())?).await?;
        assert_eq!(response.status(), status);
        if let Some(count) = count {
            let body = to_bytes(response.into_body(), 65536).await?;
            assert_eq!(serde_json::from_slice::<Vec<Value>>(&body)?.len(), count);
        }
    }
    identity.revoked.store(true, Ordering::SeqCst);
    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/runtime/capabilities")
                .header("authorization", "Bearer manager")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    drop(response);
    assert!(invocation.invoke(PATH, &Value::Null).await.is_err());
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    state.shutdown().await?;
    assert!(!invocation.available(PATH));
    assert!(state.capability_invocation(&scope(), None).is_err());
    Ok(())
}

#[tokio::test]
async fn native_calls_enforce_scope_contracts_and_do_not_retry_side_effects() -> Result<()> {
    let root = tempfile::tempdir()?;
    let identity = Arc::new(Identity::default());
    let probe = Arc::new(Probe::default());
    let state = RuntimeState::initialize(
        config(root.path()),
        plugins(identity.clone(), probe.clone()),
    )
    .await?;
    let mut denied = scope();
    denied.grants.runtime.clear();
    let invocation = state.capability_invocation(&denied, None)?;
    assert!(!invocation.available(PATH));
    assert!(invocation.invoke(PATH, &Value::Null).await.is_err());
    denied = scope();
    denied.grants.runtime.push("/v1/absent/provider".into());
    assert!(
        state
            .capability_invocation(&denied, None)?
            .invoke("/v1/absent/provider", &Value::Null)
            .await
            .is_err()
    );
    for tenant in [None, Some("".into()), Some("other".into())] {
        denied = scope();
        denied.context.tenant_id = tenant;
        assert!(
            state
                .capability_invocation(&denied, None)?
                .invoke(PATH, &Value::Null)
                .await
                .is_err()
        );
    }
    denied = scope();
    denied.context.session_id = None;
    assert!(
        state
            .capability_invocation(&denied, None)?
            .invoke(PATH, &Value::Null)
            .await
            .is_err()
    );
    denied = scope();
    denied.context.user_id = None;
    assert!(
        state
            .capability_invocation(&denied, None)?
            .invoke(PATH, &Value::Null)
            .await
            .is_err()
    );
    identity.removed.store(true, Ordering::SeqCst);
    assert!(
        state
            .capability_invocation(&scope(), None)?
            .invoke(PATH, &Value::Null)
            .await
            .is_err()
    );
    identity.removed.store(false, Ordering::SeqCst);
    let error = state
        .capability_invocation(&scope(), None)?
        .invoke(PATH, &json!({"bad":true}))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("输入不符合 schema"));
    assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    let invocation = state.capability_invocation(&scope(), None)?;
    for (input, message) in [
        ("invalid", "输出不符合 schema"),
        ("large", "超过配额"),
        ("fail", "上游失败"),
    ] {
        let before = probe.calls.load(Ordering::SeqCst);
        let error = invocation.invoke(PATH, &json!(input)).await.unwrap_err();
        assert!(error.to_string().contains(message), "{error:#}");
        assert_eq!(probe.calls.load(Ordering::SeqCst), before + 1);
    }
    state.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn native_call_chain_shares_count_transfer_and_cancellation_budgets() -> Result<()> {
    let root = tempfile::tempdir()?;
    let probe = Arc::new(Probe::default());
    let state = RuntimeState::initialize(
        config(root.path()),
        plugins(Arc::new(Identity::default()), probe.clone()),
    )
    .await?;
    let invocation = state.capability_invocation(&scope(), None)?;
    let cloned = invocation.clone();
    for index in 0..64 {
        let current = if index % 2 == 0 { &invocation } else { &cloned };
        assert_eq!(current.invoke(PATH, &Value::Null).await?, 42);
    }
    assert!(
        cloned
            .invoke(PATH, &Value::Null)
            .await
            .unwrap_err()
            .to_string()
            .contains("预算")
    );
    assert!(!invocation.available(PATH));
    let before = probe.calls.load(Ordering::SeqCst);
    let invocation = state.capability_invocation(&scope(), None)?;
    // 每次输入和输出共 65,536 字节，第 17 次必须在提供者执行前拒绝。
    let input = json!("x".repeat(65532));
    for _ in 0..16 {
        invocation.invoke(PATH, &input).await?;
    }
    assert!(invocation.clone().invoke(PATH, &input).await.is_err());
    assert_eq!(probe.calls.load(Ordering::SeqCst), before + 16);
    let invocation = state.capability_invocation(&scope(), None)?;
    assert!(
        invocation
            .invoke(PATH, &json!("x".repeat(65536)))
            .await
            .is_err()
    );
    assert_eq!(probe.calls.load(Ordering::SeqCst), before + 16);
    let invocation = state.capability_invocation(&scope(), None)?;
    invocation.clone().cancel();
    assert!(invocation.invoke(PATH, &Value::Null).await.is_err());
    assert!(
        state
            .capability_invocation(&scope(), Some(Instant::now()))
            .is_err()
    );
    state.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn deadline_cancellation_and_future_drop_stop_async_work() -> Result<()> {
    let root = tempfile::tempdir()?;
    let probe = Arc::new(Probe::default());
    let state = RuntimeState::initialize(
        config(root.path()),
        plugins(Arc::new(Identity::default()), probe.clone()),
    )
    .await?;
    for mode in ["deadline", "cancel", "drop", "shutdown"] {
        let deadline = (mode == "deadline").then(|| Instant::now() + Duration::from_millis(100));
        let invocation = state.capability_invocation(&scope(), deadline)?;
        let cloned = invocation.clone();
        let task = tokio::spawn(async move { cloned.invoke(PATH, &json!("wait")).await });
        tokio::time::timeout(Duration::from_secs(3), probe.entered.notified()).await?;
        assert_eq!(probe.active.load(Ordering::SeqCst), 1);
        match mode {
            "cancel" => invocation.cancel(),
            "drop" => task.abort(),
            "shutdown" => state.shutdown().await?,
            _ => {}
        }
        let result = tokio::time::timeout(Duration::from_secs(3), task).await?;
        if mode == "drop" {
            assert!(result.unwrap_err().is_cancelled());
        } else {
            assert!(result?.is_err());
        }
        assert_eq!(probe.active.load(Ordering::SeqCst), 0);
    }
    assert_eq!(probe.calls.load(Ordering::SeqCst), 4);
    Ok(())
}

#[tokio::test]
async fn provider_replacement_removal_and_identity_rebuild_invalidate_old_invocations() -> Result<()>
{
    let root = tempfile::tempdir()?;
    let probe = Arc::new(Probe::default());
    let mut selected = plugins(Arc::new(Identity::default()), probe.clone());
    let captured = Arc::new(Mutex::new(None));
    let store = captured.clone();
    let host = RuntimeHost::initialize_with_router(
        config(root.path()),
        selected.clone(),
        move |state, router| {
            *store
                .lock()
                .map_err(|_| anyhow::anyhow!("代数记录锁已损坏"))? = Some(state.clone());
            Ok(router)
        },
    )
    .await?;
    let state = || -> Result<RuntimeState> {
        captured
            .lock()
            .map_err(|_| anyhow::anyhow!("代数记录锁已损坏"))?
            .clone()
            .context("缺少代数")
    };
    let first = state()?.capability_invocation(&scope(), None)?;
    assert_eq!(first.invoke(PATH, &Value::Null).await?, 42);
    // 丢弃同步调用留下的通知，再确认异步提供者已经进入等待。
    probe.entered.notified().await;
    let waiting = first.clone();
    let pending = tokio::spawn(async move { waiting.invoke(PATH, &json!("wait")).await });
    tokio::time::timeout(Duration::from_secs(3), probe.entered.notified()).await?;
    selected[2] = Arc::new(ProviderPlugin(probe.clone(), 84));
    host.reconfigure(selected.clone()).await?;
    assert!(
        tokio::time::timeout(Duration::from_secs(3), pending)
            .await??
            .is_err()
    );
    assert_eq!(probe.active.load(Ordering::SeqCst), 0);
    assert!(first.invoke(PATH, &Value::Null).await.is_err());
    let second = state()?.capability_invocation(&scope(), None)?;
    assert_eq!(second.invoke(PATH, &Value::Null).await?, 84);
    // 更换身份实例也必须使旧身份快照失效。
    selected.pop();
    selected = identity_support::with_identity(Arc::new(Identity::default()), selected);
    host.reconfigure(selected.clone()).await?;
    assert!(second.invoke(PATH, &Value::Null).await.is_err());
    let third = state()?.capability_invocation(&scope(), None)?;
    assert_eq!(third.invoke(PATH, &Value::Null).await?, 84);
    selected.remove(2);
    host.reconfigure(selected.clone()).await?;
    assert!(third.invoke(PATH, &Value::Null).await.is_err());
    let missing = state()?.capability_invocation(&scope(), None)?;
    assert!(!missing.available(PATH));
    assert!(missing.invoke(PATH, &Value::Null).await.is_err());
    selected.push(Arc::new(ProviderPlugin(probe, 126)));
    host.reconfigure(selected).await?;
    assert!(!missing.available(PATH));
    assert_eq!(
        state()?
            .capability_invocation(&scope(), None)?
            .invoke(PATH, &Value::Null)
            .await?,
        126
    );
    host.shutdown().await?;
    Ok(())
}
