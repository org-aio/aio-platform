//! 执行器暂缺时保留安装意图；在线恢复与租户管理走真实 HTTP 入口。

use super::fixture::*;
use anyhow::{Context, Result};
use az_plugin_core::{Plugin, PluginFuture};
use az_plugin_host::runtime::server::plugins::*;
use serde_json::{Value, json};
use std::{any::TypeId, sync::Arc};
use uuid::Uuid;

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn missing_executors_suspend_installed_components_and_preserve_management() -> Result<()> {
    let database = database().await?;
    let root = tempfile::tempdir()?;
    let tenant = Uuid::new_v4().to_string();
    let git = format!("https://github.com/example/availability-{tenant}.git");
    let probes: [Arc<Probe>; 2] = Default::default();
    let first = permission_package(root.path(), &git, 1, "1.0.0")?;
    let second = permission_package(root.path(), &git, 2, "2.0.0")?;
    let mut host = Host::start(root.path(), &database, &tenant, probes.clone()).await?;
    let source: Uuid = success(host.publish(&first).await?).await?["data"]["source_id"]
        .as_str()
        .context("来源缺失")?
        .parse()?;
    success(host.install(&first).await?).await?;
    success(host.publish(&second).await?).await?;
    let token = host.assert_call(source, 1, &first).await?;
    assert_state(&host, source, "active", true).await?;
    let before = installation(&host, &tenant, source).await?;
    let full = host.selected.clone();
    let empty = full
        .iter()
        .filter(|entry| entry.as_ref().type_id() != TypeId::of::<Replacements>())
        .cloned()
        .collect::<Vec<_>>();

    host.reconfigure(empty.clone()).await?;
    assert_eq!(installation(&host, &tenant, source).await?, before);
    assert_state(&host, source, "suspended", false).await?;
    assert!(host.mount(source).await.is_err());
    assert!(!host.invoke(&token).await?.status().is_success());
    for probe in &probes {
        assert!(probe.active.lock().await.is_empty());
    }
    assert!(!host.install(&second).await?.status().is_success());
    assert_eq!(installation(&host, &tenant, source).await?, before);

    host.reconfigure(full.clone()).await?;
    assert_eq!(installation(&host, &tenant, source).await?, before);
    host.assert_call(source, 1, &first).await?;
    assert_state(&host, source, "active", true).await?;

    host.reconfigure(empty.clone()).await?;
    success(
        host.post(&format!("/api/runtime/plugins/{source}/disable"), json!({}))
            .await?,
    )
    .await?;
    assert_state(&host, source, "disabled", false).await?;
    host.reconfigure(full).await?;
    assert_state(&host, source, "disabled", false).await?;
    success(
        host.post(&format!("/api/runtime/plugins/{source}/enable"), json!({}))
            .await?,
    )
    .await?;
    host.assert_call(source, 1, &first).await?;
    host.reconfigure(empty).await?;
    success(
        host.post(
            &format!("/api/runtime/plugins/{source}/uninstall"),
            json!({}),
        )
        .await?,
    )
    .await?;
    host.stop().await?;
    Ok(())
}

async fn installation(host: &Host, tenant: &str, source: Uuid) -> Result<Value> {
    Ok(sqlx::query_scalar(
        "SELECT to_jsonb(i) FROM component_installations i WHERE tenant_id=$1 AND source_id=$2",
    )
    .bind(tenant)
    .bind(source)
    .fetch_one(&*host.state.service::<sqlx::PgPool>()?)
    .await?)
}

async fn assert_state(host: &Host, source: Uuid, state: &str, visible: bool) -> Result<()> {
    let catalog = success(
        host.client
            .get(format!("{}/api/runtime/catalog", host.base))
            .send()
            .await?,
    )
    .await?;
    let data = &catalog["data"];
    let source = source.to_string();
    let installed = data["plugins"]
        .as_array()
        .context("没有安装目录")?
        .iter()
        .find(|plugin| plugin["source_id"] == source)
        .context("没有已安装插件")?;
    assert_eq!(installed["state"], state);
    assert_eq!(
        data["pages"]
            .as_array()
            .context("没有页面目录")?
            .iter()
            .any(|page| page["id"] == format!("component:{source}:home")),
        visible
    );
    let entries = success(
        host.client
            .get(format!("{}/api/runtime/marketplace", host.base))
            .send()
            .await?,
    )
    .await?;
    let entry = entries["data"]
        .as_array()
        .context("没有市场目录")?
        .iter()
        .find(|entry| entry["source_id"] == source)
        .context("市场丢失了已安装项")?;
    assert_eq!(entry["state"], state);
    let snapshot = success(
        host.client
            .get(format!("{}/api/runtime/bootstrap", host.base))
            .send()
            .await?,
    )
    .await?;
    assert_eq!(
        snapshot["data"]["permissions"]
            .as_array()
            .context("没有权限列表")?
            .iter()
            .any(|permission| permission == &format!("component:{source}:fixture.view")),
        visible
    );
    assert_eq!(
        data["plugin_settings"]
            .as_array()
            .context("没有设置目录")?
            .iter()
            .any(|item| item["source_id"] == source),
        visible
    );
    Ok(())
}

fn permission_package(
    root: &std::path::Path,
    git: &str,
    executor: u8,
    version: &str,
) -> Result<az_plugin_bundle::Bundle> {
    package(root, git, executor, version)?;
    let directory = root.join(version);
    let manifest = directory.join("aio-plugin.toml");
    let text = std::fs::read_to_string(&manifest)?.replace(
        "[plugin.marketplace]",
        "[plugin]\npermissions=['fixture.view']\nsettings_page='home'\n[plugin.marketplace]",
    );
    std::fs::write(manifest, text)?;
    az_plugin_bundle::Bundle::from_directory(
        &directory,
        "aio-plugin.toml",
        git.into(),
        "a".repeat(40),
        version.into(),
    )
}

struct SecondOnly(Arc<Probe>);

impl Plugin<RuntimeAssembly> for SecondOnly {
    fn build<'a>(&'a self, assembly: &'a mut RuntimeAssembly) -> PluginFuture<'a> {
        Box::pin(async move {
            assembly
                .services
                .add_value(Replacement::<2>(self.0.clone()))
                .bind::<dyn ComponentExecutor, Replacement<2>>();
            Ok(())
        })
    }
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn missing_previous_executor_allows_migration_and_commit_failure_keeps_suspension()
-> Result<()> {
    let database = database().await?;
    let root = tempfile::tempdir()?;
    let tenant = Uuid::new_v4().to_string();
    let git = format!("https://github.com/example/migration-{tenant}.git");
    let probes: [Arc<Probe>; 2] = Default::default();
    let first = permission_package(root.path(), &git, 1, "1.0.0")?;
    let second = permission_package(root.path(), &git, 2, "2.0.0")?;
    let mut host = Host::start(root.path(), &database, &tenant, probes.clone()).await?;
    let source: Uuid = success(host.publish(&first).await?).await?["data"]["source_id"]
        .as_str()
        .context("没有来源")?
        .parse()?;
    success(host.install(&first).await?).await?;
    success(host.publish(&second).await?).await?;
    let before = installation(&host, &tenant, source).await?;
    let mut partial = host.selected.clone();
    partial.retain(|entry| entry.as_ref().type_id() != TypeId::of::<Replacements>());
    partial.push(Arc::new(SecondOnly(probes[1].clone())));
    host.reconfigure(partial).await?;
    assert_state(&host, source, "suspended", false).await?;
    let pool = host.state.service::<sqlx::PgPool>()?;
    for action in ["disable", "uninstall"] {
        committed_failure(&pool, &first).await?;
        let failed = host
            .post(
                &format!("/api/runtime/plugins/{source}/{action}"),
                json!({}),
            )
            .await?;
        clear_failure(&pool).await?;
        assert!(!failed.status().is_success());
        assert_eq!(installation(&host, &tenant, source).await?, before);
    }
    committed_failure(&pool, &second).await?;
    let failed = host.install(&second).await?;
    clear_failure(&pool).await?;
    assert!(!failed.status().is_success());
    assert_eq!(installation(&host, &tenant, source).await?, before);
    assert_state(&host, source, "suspended", false).await?;
    assert!(probes[1].active.lock().await.is_empty());
    success(host.install(&second).await?).await?;
    host.assert_call(source, 2, &second).await?;
    assert_state(&host, source, "active", true).await?;
    success(
        host.post(
            &format!("/api/runtime/plugins/{source}/uninstall"),
            json!({}),
        )
        .await?,
    )
    .await?;
    host.stop().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn missing_executor_does_not_hide_corrupted_persisted_bundles() -> Result<()> {
    let database = database().await?;
    let root = tempfile::tempdir()?;
    let tenant = Uuid::new_v4().to_string();
    let git = format!("https://github.com/example/integrity-{tenant}.git");
    let first = permission_package(root.path(), &git, 1, "1.0.0")?;
    let other = permission_package(root.path(), &git, 2, "2.0.0")?;
    let mut host = Host::start(root.path(), &database, &tenant, Default::default()).await?;
    let source: Uuid = success(host.publish(&first).await?).await?["data"]["source_id"]
        .as_str()
        .context("没有来源")?
        .parse()?;
    success(host.install(&first).await?).await?;
    let pool = sqlx::PgPool::connect(&database).await?;
    sqlx::query("UPDATE component_versions SET archive=$2 WHERE digest=$1")
        .bind(&first.digest)
        .bind(other.encode()?)
        .execute(&pool)
        .await?;
    let mut empty = host.selected.clone();
    empty.retain(|entry| entry.as_ref().type_id() != TypeId::of::<Replacements>());
    let failed = host.reconfigure(empty).await;
    // 先修复独立测试库，失败断言也不把损坏数据留给其余测试。
    sqlx::query("UPDATE component_versions SET archive=$2 WHERE digest=$1")
        .bind(&first.digest)
        .bind(first.encode()?)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM component_installations WHERE source_id=$1")
        .bind(source)
        .execute(&pool)
        .await?;
    let error = failed.err().context("缺失执行器掩盖了摘要损坏")?;
    assert!(
        format!("{error:#}").contains("安装记录与整包摘要不匹配"),
        "{error:#}"
    );
    host.stop().await?;
    pool.close().await;
    Ok(())
}
