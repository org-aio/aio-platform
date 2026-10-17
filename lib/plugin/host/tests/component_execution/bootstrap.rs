use std::{
    any::TypeId,
    path::Path,
    sync::{Arc, atomic::Ordering},
};

use anyhow::{Context, Result};
use az_plugin_bundle::Bundle;
use az_plugin_core::DynPlugin;
use az_plugin_host::runtime::server::{RuntimeState, plugins::*};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::{fixture::*, identity_support};

fn declaration(root: &Path, bundle: &Bundle, tenants: &[&str]) -> Result<BootstrapComponent> {
    let path = root.join(format!("{}.aio", bundle.digest));
    std::fs::write(&path, bundle.encode()?)?;
    Ok(BootstrapComponent {
        path,
        digest: bundle.digest.clone(),
        tenants: tenants.iter().map(|tenant| (*tenant).into()).collect(),
    })
}

fn selected(
    tenant: &str,
    probes: [Arc<Probe>; 2],
    declarations: Vec<BootstrapComponent>,
    executors: bool,
) -> Vec<DynPlugin<RuntimeAssembly>> {
    let mut selected = plugins(probes, true);
    if !executors {
        selected.retain(|plugin| plugin.as_ref().type_id() != TypeId::of::<Replacements>());
    }
    selected.push(Arc::new(ComponentBootstrapPlugin(declarations)));
    identity_support::with_identity(Arc::new(Identity(tenant.into())), selected)
}

async fn source(pool: &PgPool, git: &str) -> Result<Uuid> {
    Ok(
        sqlx::query_scalar("SELECT id FROM component_sources WHERE git=$1")
            .bind(git)
            .fetch_one(pool)
            .await?,
    )
}

async fn installation(pool: &PgPool, tenant: &str, source: Uuid) -> Result<serde_json::Value> {
    Ok(sqlx::query_scalar(
        "SELECT row_to_json(i) FROM component_installations i WHERE tenant_id=$1 AND source_id=$2",
    )
    .bind(tenant)
    .bind(source)
    .fetch_one(pool)
    .await?)
}

async fn initialized(pool: &PgPool, tenant: &str, git: &str) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM component_initializations WHERE tenant_id=$1 AND git=$2)",
    )
    .bind(tenant)
    .bind(git)
    .fetch_one(pool)
    .await?)
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn bootstrap_preserves_publication_rollback_disable_uninstall_and_backfills_intent()
-> Result<()> {
    let database = database().await?;
    let pool = PgPool::connect(&database).await?;
    let root = tempfile::tempdir()?;
    let tenant = Uuid::new_v4().to_string();
    let git = format!("https://github.com/example/bootstrap-{tenant}.git");
    let probes = [Arc::default(), Arc::default()];
    let first = package(root.path(), &git, 1, "1.0.0")?;
    let second = package(root.path(), &git, 2, "2.0.0")?;
    let declaration = declaration(root.path(), &first, &[&tenant])?;
    let host = Host::start(root.path(), &database, &tenant, probes.clone()).await?;
    success(host.publish(&first).await?).await?;
    success(host.publish(&second).await?).await?;
    let id = source(&pool, &git).await?;
    host.stop().await?;

    let selection = selected(&tenant, probes.clone(), vec![declaration], true);
    let host = Host::start_selected(root.path(), &database, selection.clone()).await?;
    host.assert_call(id, 1, &first).await?;
    let published: String =
        sqlx::query_scalar("SELECT digest FROM component_publications WHERE source_id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(published, second.digest);
    assert!(initialized(&pool, &tenant, &git).await?);
    assert!(!initialized(&pool, "not-declared", &git).await?);
    success(host.install(&second).await?).await?;
    success(
        host.post(&format!("/api/runtime/plugins/{id}/rollback"), json!({}))
            .await?,
    )
    .await?;
    let rolled_back = installation(&pool, &tenant, id).await?;
    assert_eq!(rolled_back["excluded_digest"], second.digest);
    host.stop().await?;

    let host = Host::start_selected(root.path(), &database, selection.clone()).await?;
    assert_eq!(installation(&pool, &tenant, id).await?, rolled_back);
    host.assert_call(id, 1, &first).await?;
    success(
        host.post(&format!("/api/runtime/plugins/{id}/disable"), json!({}))
            .await?,
    )
    .await?;
    let disabled = installation(&pool, &tenant, id).await?;
    // 模拟升级前已有的停用安装，启动迁移必须回填历史意图。
    sqlx::query("DELETE FROM component_initializations WHERE tenant_id=$1 AND git=$2")
        .bind(&tenant)
        .bind(&git)
        .execute(&pool)
        .await?;
    host.stop().await?;
    let host = Host::start_selected(root.path(), &database, selection.clone()).await?;
    assert_eq!(installation(&pool, &tenant, id).await?, disabled);
    assert!(initialized(&pool, &tenant, &git).await?);
    assert!(host.mount(id).await.is_err());
    success(
        host.post(&format!("/api/runtime/plugins/{id}/uninstall"), json!({}))
            .await?,
    )
    .await?;
    host.stop().await?;
    let host = Host::start_selected(root.path(), &database, selection.clone()).await?;
    assert!(installation(&pool, &tenant, id).await.is_err());
    assert!(initialized(&pool, &tenant, &git).await?);
    host.stop().await?;
    // 来源删除也不能清除租户曾明确安装、随后卸载的选择。
    for table in ["component_publications", "component_versions"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE source_id=$1"))
            .bind(id)
            .execute(&pool)
            .await?;
    }
    sqlx::query("DELETE FROM component_sources WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await?;
    let host = Host::start_selected(root.path(), &database, selection).await?;
    assert!(source(&pool, &git).await.is_err());
    host.stop().await?;
    pool.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn bootstrap_waits_for_executor_and_resumes_on_live_assembly() -> Result<()> {
    let database = database().await?;
    let root = tempfile::tempdir()?;
    let tenant = Uuid::new_v4().to_string();
    let git = format!("https://github.com/example/bootstrap-wait-{tenant}.git");
    let probes = [Arc::default(), Arc::default()];
    let bundle = package(root.path(), &git, 1, "1.0.0")?;
    let declaration = declaration(root.path(), &bundle, &[&tenant])?;
    let mut host = Host::start_selected(
        root.path(),
        &database,
        selected(&tenant, probes.clone(), vec![declaration.clone()], false),
    )
    .await?;
    let pool = PgPool::connect(&database).await?;
    assert!(!initialized(&pool, &tenant, &git).await?);
    assert!(source(&pool, &git).await.is_err());
    let mut selection = host.selected.clone();
    selection.push(Arc::new(Replacements(probes.clone())));
    host.reconfigure(selection).await?;
    let id = source(&pool, &git).await?;
    host.assert_call(id, 1, &bundle).await?;
    let installed = installation(&pool, &tenant, id).await?;
    let mut selection = host.selected.clone();
    selection.retain(|plugin| plugin.as_ref().type_id() != TypeId::of::<Replacements>());
    host.reconfigure(selection).await?;
    assert_eq!(installation(&pool, &tenant, id).await?, installed);
    host.stop().await?;
    let host = Host::start_selected(
        root.path(),
        &database,
        selected(&tenant, probes, vec![declaration], true),
    )
    .await?;
    host.assert_call(id, 1, &bundle).await?;
    assert_eq!(installation(&pool, &tenant, id).await?, installed);
    success(
        host.post(&format!("/api/runtime/plugins/{id}/uninstall"), json!({}))
            .await?,
    )
    .await?;
    host.stop().await?;
    pool.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn bootstrap_commit_failure_keeps_completed_items_and_retries_uncommitted_item() -> Result<()>
{
    let database = database().await?;
    let pool = PgPool::connect(&database).await?;
    let root = tempfile::tempdir()?;
    let tenant = Uuid::new_v4().to_string();
    let probes = [Arc::default(), Arc::default()];
    let first = package(
        root.path(),
        &format!("https://github.com/example/bootstrap-first-{tenant}.git"),
        1,
        "1.0.0",
    )?;
    let second = package(
        root.path(),
        &format!("https://github.com/example/bootstrap-second-{tenant}.git"),
        2,
        "2.0.0",
    )?;
    let declarations = vec![
        declaration(root.path(), &first, &[&tenant])?,
        declaration(root.path(), &second, &[&tenant])?,
    ];
    let mut host = Host::start(root.path(), &database, &tenant, probes.clone()).await?;
    committed_failure(&pool, &second).await?;
    let mut selection = host.selected.clone();
    selection.push(Arc::new(ComponentBootstrapPlugin(declarations)));
    let failed = host.reconfigure(selection.clone()).await;
    clear_failure(&pool).await?;
    assert!(failed.is_err());
    assert!(initialized(&pool, &tenant, &first.git).await?);
    assert!(!initialized(&pool, &tenant, &second.git).await?);
    let first_id = source(&pool, &first.git).await?;
    let installed = installation(&pool, &tenant, first_id).await?;
    host.assert_call(first_id, 1, &first).await?;
    assert!(probes[1].active.lock().await.is_empty());
    host.reconfigure(selection).await?;
    assert_eq!(installation(&pool, &tenant, first_id).await?, installed);
    let second_id = source(&pool, &second.git).await?;
    host.assert_call(second_id, 2, &second).await?;
    assert!(initialized(&pool, &tenant, &second.git).await?);
    for id in [first_id, second_id] {
        success(
            host.post(&format!("/api/runtime/plugins/{id}/uninstall"), json!({}))
                .await?,
        )
        .await?;
    }
    host.stop().await?;
    pool.close().await;
    Ok(())
}

fn parent_package(root: &Path, git: &str, parent: &str, version: &str) -> Result<Bundle> {
    package(root, git, 1, version)?;
    let directory = root.join(version);
    let path = directory.join("aio-plugin.toml");
    let manifest = std::fs::read_to_string(&path)?;
    std::fs::write(
        path,
        manifest.replace(
            "title='Executor'",
            &format!("title='Executor'\nparent='{parent}'\nparent_title='Parent'"),
        ),
    )?;
    Bundle::from_directory(
        &directory,
        "aio-plugin.toml",
        git.into(),
        "a".repeat(40),
        version.into(),
    )
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn bootstrap_executor_validation_and_ambiguity_fail_before_installations() -> Result<()> {
    let database = database().await?;
    let pool = PgPool::connect(&database).await?;
    let root = tempfile::tempdir()?;
    let tenant = Uuid::new_v4().to_string();
    let probes: [Arc<Probe>; 2] = Default::default();
    let first = package(
        root.path(),
        &format!("https://github.com/example/bootstrap-preflight-{tenant}.git"),
        1,
        "1.0.0",
    )?;
    let second = package(
        root.path(),
        &format!("https://github.com/example/bootstrap-invalid-{tenant}.git"),
        2,
        "2.0.0",
    )?;
    let declarations = vec![
        declaration(root.path(), &first, &[&tenant])?,
        declaration(root.path(), &second, &[&tenant])?,
    ];
    let mut host = Host::start(root.path(), &database, &tenant, probes.clone()).await?;
    let mut selection = host.selected.clone();
    selection.push(Arc::new(ComponentBootstrapPlugin(declarations)));
    probes[1].fail_validate.store(true, Ordering::SeqCst);
    let failed = host
        .reconfigure(selection.clone())
        .await
        .err()
        .context("接受了验证失败的批次")?;
    assert!(format!("{failed:#}").contains("测试候选验证失败"));
    assert!(!initialized(&pool, &tenant, &first.git).await?);
    assert!(!initialized(&pool, &tenant, &second.git).await?);
    probes[1].fail_validate.store(false, Ordering::SeqCst);
    probes[1].accept_all.store(true, Ordering::SeqCst);
    let failed = host
        .reconfigure(selection.clone())
        .await
        .err()
        .context("接受了歧义执行器")?;
    assert!(format!("{failed:#}").contains("多个执行器"));
    assert!(!initialized(&pool, &tenant, &first.git).await?);
    probes[1].accept_all.store(false, Ordering::SeqCst);
    host.reconfigure(selection).await?;
    for (bundle, executor) in [(&first, 1), (&second, 2)] {
        let id = source(&pool, &bundle.git).await?;
        host.assert_call(id, executor, bundle).await?;
        success(
            host.post(&format!("/api/runtime/plugins/{id}/uninstall"), json!({}))
                .await?,
        )
        .await?;
    }
    host.stop().await?;
    pool.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn bootstrap_orders_parents_and_preserves_disabled_parent_choice() -> Result<()> {
    let database = database().await?;
    let pool = PgPool::connect(&database).await?;
    let root = tempfile::tempdir()?;
    let tenant = Uuid::new_v4().to_string();
    let probes = [Arc::default(), Arc::default()];
    let parent = package(
        root.path(),
        &format!("https://github.com/example/bootstrap-parent-{tenant}.git"),
        1,
        "1.0.0",
    )?;
    let child = parent_package(
        root.path(),
        &format!("https://github.com/example/bootstrap-child-{tenant}.git"),
        &parent.git,
        "2.0.0",
    )?;
    let declarations = vec![
        declaration(root.path(), &child, &[&tenant])?,
        declaration(root.path(), &parent, &[&tenant])?,
    ];
    let host = Host::start_selected(
        root.path(),
        &database,
        selected(&tenant, probes.clone(), declarations, true),
    )
    .await?;
    let parent_id = source(&pool, &parent.git).await?;
    let child_id = source(&pool, &child.git).await?;
    host.assert_call(parent_id, 1, &parent).await?;
    host.assert_call(child_id, 1, &child).await?;
    success(
        host.post(
            &format!("/api/runtime/plugins/{child_id}/uninstall"),
            json!({}),
        )
        .await?,
    )
    .await?;
    success(
        host.post(
            &format!("/api/runtime/plugins/{parent_id}/disable"),
            json!({}),
        )
        .await?,
    )
    .await?;
    let disabled = installation(&pool, &tenant, parent_id).await?;
    host.stop().await?;
    let newcomer = parent_package(
        root.path(),
        &format!("https://github.com/example/bootstrap-new-{tenant}.git"),
        &parent.git,
        "3.0.0",
    )?;
    let declarations = vec![
        declaration(root.path(), &parent, &[&tenant])?,
        declaration(root.path(), &newcomer, &[&tenant])?,
    ];
    let mut host = Host::start_selected(
        root.path(),
        &database,
        selected(&tenant, probes.clone(), declarations.clone(), true),
    )
    .await?;
    assert_eq!(installation(&pool, &tenant, parent_id).await?, disabled);
    assert!(!initialized(&pool, &tenant, &newcomer.git).await?);
    success(
        host.post(
            &format!("/api/runtime/plugins/{parent_id}/enable"),
            json!({}),
        )
        .await?,
    )
    .await?;
    host.reconfigure(selected(&tenant, probes, declarations, true))
        .await?;
    host.assert_call(source(&pool, &newcomer.git).await?, 1, &newcomer)
        .await?;
    assert!(installation(&pool, &tenant, child_id).await.is_err());
    for id in [source(&pool, &newcomer.git).await?, parent_id] {
        success(
            host.post(&format!("/api/runtime/plugins/{id}/uninstall"), json!({}))
                .await?,
        )
        .await?;
    }
    host.stop().await?;
    pool.close().await;
    Ok(())
}

#[tokio::test]
async fn bootstrap_rejects_bad_batches_before_initializing_resources() -> Result<()> {
    let root = tempfile::tempdir()?;
    let first = package(
        root.path(),
        "https://github.com/example/bootstrap-valid.git",
        1,
        "1.0.0",
    )?;
    let valid = declaration(root.path(), &first, &["tenant"])?;
    let mut wrong_digest = valid.clone();
    wrong_digest.digest = "0".repeat(64);
    let mut no_tenants = valid.clone();
    no_tenants.tenants.clear();
    let mut repeated_tenants = valid.clone();
    repeated_tenants.tenants.push("tenant".into());
    let mut directory = valid.clone();
    directory.path = root.path().into();
    let cycle_a = parent_package(
        root.path(),
        "https://github.com/example/cycle-a.git",
        "https://github.com/example/cycle-b.git",
        "2.0.0",
    )?;
    let cycle_b = parent_package(
        root.path(),
        "https://github.com/example/cycle-b.git",
        "https://github.com/example/cycle-a.git",
        "3.0.0",
    )?;
    let privileged_root = root.path().join("4.0.0");
    package(
        root.path(),
        "https://github.com/example/privileged.git",
        1,
        "4.0.0",
    )?;
    let path = privileged_root.join("aio-plugin.toml");
    let manifest = std::fs::read_to_string(&path)?;
    std::fs::write(
        path,
        format!("{manifest}\n[plugin.capabilities]\nmanagement=true\n"),
    )?;
    let privileged = Bundle::from_directory(
        &privileged_root,
        "aio-plugin.toml",
        "https://github.com/example/privileged.git".into(),
        "a".repeat(40),
        "4.0.0".into(),
    )?;
    let cases = [
        (vec![valid.clone(), wrong_digest], "摘要不匹配"),
        (vec![valid.clone(), no_tenants], "租户"),
        (vec![valid.clone(), repeated_tenants], "租户"),
        (vec![valid.clone(), valid.clone()], "来源重复"),
        (vec![valid.clone(), directory], "普通文件"),
        (
            vec![
                declaration(root.path(), &cycle_a, &["tenant"])?,
                declaration(root.path(), &cycle_b, &["tenant"])?,
            ],
            "循环",
        ),
        (
            vec![valid, declaration(root.path(), &privileged, &["tenant"])?],
            "宿主管理",
        ),
    ];
    for (declarations, expected) in cases {
        let probes: [Arc<Probe>; 2] = Default::default();
        let result = RuntimeState::initialize(
            config(root.path(), "invalid database", "http://127.0.0.1"),
            selected("tenant", probes.clone(), declarations, true),
        )
        .await;
        let error = result.err().context("无效批次被接受")?;
        assert!(format!("{error:#}").contains(expected), "{error:#}");
        assert!(probes[0].events.lock().await.is_empty());
        assert!(probes[1].events.lock().await.is_empty());
    }
    assert!(!root.path().join("components").exists());
    Ok(())
}
