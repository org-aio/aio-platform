#![cfg(feature = "server")]

#[path = "support/identity.rs"]
mod identity_support;

#[path = "component_execution/fixture.rs"]
mod fixture;

#[path = "component_execution/resources.rs"]
mod resources;

#[path = "component_execution/access.rs"]
mod access;

#[path = "component_execution/availability.rs"]
mod availability;

#[path = "component_execution/bootstrap.rs"]
mod bootstrap;

#[path = "component_execution/management.rs"]
mod management;

#[path = "component_execution/legacy_availability.rs"]
mod legacy_availability;

#[path = "component_execution/legacy_executor.rs"]
mod legacy_executor;

#[path = "component_execution/legacy_validation.rs"]
mod legacy_validation;

#[path = "component_execution/legacy_development.rs"]
mod legacy_development;

use anyhow::{Context, Result};
use az_plugin_host::runtime::server::{RuntimeState, plugins::*};
use fixture::*;
use serde_json::json;
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use uuid::Uuid;

#[tokio::test]
async fn component_registry_accepts_empty_and_rejects_duplicate_types_without_resources()
-> Result<()> {
    let empty = dill::Catalog::builder()
        .add::<ComponentExecutors>()
        .build()
        .get::<dill::OneOf<ComponentExecutors>>()?;
    empty.validate()?;
    let duplicate = dill::Catalog::builder()
        .add_value(Replacement::<1>(Arc::default()))
        .bind::<dyn ComponentExecutor, Replacement<1>>()
        .bind::<dyn ComponentExecutor, Replacement<1>>()
        .add::<ComponentExecutors>()
        .build()
        .get::<dill::OneOf<ComponentExecutors>>()?;
    assert!(duplicate.validate().is_err());
    let root = tempfile::tempdir()?;
    let state = RuntimeState::initialize(
        config(root.path(), "invalid database", "http://127.0.0.1"),
        identity_support::with_identity(
            Arc::new(Identity("test".into())),
            vec![
                Arc::new(ComponentExecutionPlugin),
                Arc::new(Replacements(Default::default())),
            ],
        ),
    )
    .await?;
    state.service::<ComponentExecutors>()?.validate()?;
    assert!(!root.path().join("components").exists());
    state.shutdown().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn external_components_cover_http_switch_rollback_commit_failure_and_restart() -> Result<()> {
    let database = database().await?;
    let root = tempfile::tempdir()?;
    let tenant = Uuid::new_v4().to_string();
    let git = format!("https://github.com/example/executors-{tenant}.git");
    let probes: [Arc<Probe>; 2] = Default::default();
    let first = package(root.path(), &git, 1, "1.0.0")?;
    let second = package(root.path(), &git, 2, "2.0.0")?;
    let host = Host::start(root.path(), &database, &tenant, probes.clone()).await?;
    let pool = host.state.service::<sqlx::PgPool>()?;
    let published = success(host.publish(&first).await?).await?;
    let source: Uuid = published["data"]["source_id"]
        .as_str()
        .context("来源缺失")?
        .parse()?;
    success(host.install(&first).await?).await?;
    let old_token = host.assert_call(source, 1, &first).await?;
    assert!(
        !host
            .client
            .post(format!(
                "{}/api/runtime/components/{old_token}/request",
                host.base
            ))
            .header("cookie", "session=other")
            .json(&json!({"method":"GET","path":"/test"}))
            .send()
            .await?
            .status()
            .is_success()
    );

    let unsupported = package(root.path(), &git, 3, "3.0.0")?;
    assert!(!host.publish(&unsupported).await?.status().is_success());
    probes[1].accept_all.store(true, Ordering::SeqCst);
    assert!(!host.publish(&first).await?.status().is_success());
    probes[1].accept_all.store(false, Ordering::SeqCst);
    success(host.publish(&second).await?).await?;

    probes[1].fail_activate.store(true, Ordering::SeqCst);
    probes[1].fail_deactivate.store(true, Ordering::SeqCst);
    let failed = host.install(&second).await?;
    assert!(!failed.status().is_success());
    let detail = failed.text().await?;
    assert!(
        detail.contains("激活失败") && detail.contains("清理候选失败"),
        "{detail}"
    );
    assert!(probes[0].events.lock().await.contains(&"restore"));
    host.assert_call(source, 1, &first).await?;
    probes[1].fail_activate.store(false, Ordering::SeqCst);
    probes[1].fail_deactivate.store(false, Ordering::SeqCst);

    committed_failure(&pool, &second).await?;
    let failed = host.install(&second).await?;
    clear_failure(&pool).await?;
    assert!(!failed.status().is_success());
    host.assert_call(source, 1, &first).await?;
    assert!(probes[1].active.lock().await.is_empty());

    success(host.install(&second).await?).await?;
    host.assert_call(source, 2, &second).await?;
    assert!(!host.invoke(&old_token).await?.status().is_success());
    assert!(probes[0].active.lock().await.is_empty());
    success(
        host.post(
            &format!("/api/runtime/plugins/{source}/rollback"),
            json!({}),
        )
        .await?,
    )
    .await?;
    host.assert_call(source, 1, &first).await?;
    success(host.install(&second).await?).await?;

    let third = package(root.path(), &git, 2, "4.0.0")?;
    success(host.publish(&third).await?).await?;
    committed_failure(&pool, &third).await?;
    let failed = host.install(&third).await?;
    clear_failure(&pool).await?;
    assert!(!failed.status().is_success());
    host.assert_call(source, 2, &second).await?;
    // 最新发布保持 second，便于验证停用后的 enable 语义。
    success(host.publish(&second).await?).await?;
    assert!(!root.path().join("components/compiled").exists());
    assert!(!root.path().join("components/processes").exists());
    host.stop().await?;
    assert!(pool.is_closed());
    for probe in &probes {
        assert!(probe.active.lock().await.is_empty());
    }

    let host = Host::start(root.path(), &database, &tenant, probes.clone()).await?;
    host.assert_call(source, 2, &second).await?;
    assert!(probes[1].events.lock().await.contains(&"restore"));
    let pool = host.state.service::<sqlx::PgPool>()?;
    for action in ["disable", "uninstall"] {
        committed_failure(&pool, &second).await?;
        let failed = host
            .post(
                &format!("/api/runtime/plugins/{source}/{action}"),
                json!({}),
            )
            .await?;
        clear_failure(&pool).await?;
        assert!(!failed.status().is_success());
        host.assert_call(source, 2, &second).await?;
    }
    for action in ["disable", "enable", "uninstall"] {
        success(
            host.post(
                &format!("/api/runtime/plugins/{source}/{action}"),
                json!({}),
            )
            .await?,
        )
        .await?;
        if action == "enable" {
            host.assert_call(source, 2, &second).await?;
        } else {
            assert!(host.mount(source).await.is_err());
        }
    }
    host.stop().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn executor_initialization_failure_panic_and_shutdown_panic_release_all_resources()
-> Result<()> {
    let database = database().await?;
    for failure in [0, 1, 2] {
        let root = tempfile::tempdir()?;
        let probes: [Arc<Probe>; 2] = Default::default();
        probes[1].initialization.store(failure, Ordering::SeqCst);
        probes[1].panic_stop.store(true, Ordering::SeqCst);
        let result = RuntimeState::initialize(
            config(root.path(), &database, "http://127.0.0.1"),
            identity_support::with_identity(
                Arc::new(Identity("test".into())),
                plugins(probes.clone(), false),
            ),
        )
        .await;
        if failure == 0 {
            assert!(result?.shutdown().await.is_err());
        } else {
            assert!(result.is_err());
        }
        for probe in &probes {
            assert_eq!(probe.stopped.load(Ordering::SeqCst), 1);
            assert!(
                probe
                    .pool
                    .lock()
                    .await
                    .as_ref()
                    .context("没有注入共享连接池")?
                    .is_closed()
            );
        }
        assert!(!root.path().join("components/compiled").exists());
        assert!(!root.path().join("components/processes").exists());
    }
    Ok(())
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn cancelled_startup_finishes_executor_cleanup_at_the_lifecycle_boundary() -> Result<()> {
    let database = database().await?;
    let root = tempfile::tempdir()?;
    let probes: [Arc<Probe>; 2] = Default::default();
    probes[1].initialization.store(3, Ordering::SeqCst);
    let configuration = config(root.path(), &database, "http://127.0.0.1");
    let selected = plugins(probes.clone(), false);
    tokio::task::LocalSet::new()
        .run_until(async move {
            let task = tokio::task::spawn_local(async move {
                RuntimeState::initialize(
                    configuration,
                    identity_support::with_identity(Arc::new(Identity("test".into())), selected),
                )
                .await
            });
            tokio::time::timeout(Duration::from_secs(10), probes[1].entered.notified()).await?;
            task.abort();
            let _ = task.await;
            probes[1].resume.notify_one();
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let pool_closed = probes[0]
                        .pool
                        .lock()
                        .await
                        .as_ref()
                        .is_some_and(|pool| pool.is_closed());
                    if probes
                        .iter()
                        .all(|probe| probe.stopped.load(Ordering::SeqCst) == 1)
                        && pool_closed
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await?;
            Ok(())
        })
        .await
}
