//! 发布推进与租户自动升级分别装配；两者不要求 GitHub 发现器。
use super::super::fixture::{database, package, plugins};
use super::delivery_http::{build_request, claim};
use super::*;
use az_plugin_bundle::Bundle;
use az_plugin_delivery::{BuildJob, BuildReport, Documentation};
use az_plugin_host::{
    configuration::DeliveryConfig,
    runtime::server::{delivery::*, plugins::ComponentCatalog},
};

async fn submit(
    host: &Host,
    root: &std::path::Path,
    git: &str,
    revision: char,
    executor: u8,
) -> Result<Bundle> {
    let sources = host.state.service::<dyn DeliverySources>()?;
    let current = sources.get(git).await?.map(|s| s.generation);
    assert!(
        sources
            .update(
                git,
                current,
                Some(DeliveryTarget {
                    branch: "main".into(),
                    source_revision: revision.to_string().repeat(40),
                    recipe: serde_json::from_value(
                        json!({"environment":"rust","command":["fixture"]})
                    )?
                })
            )
            .await?
    );
    let job: BuildJob = claim(host).await?;
    package(root, git, executor, &job.version)?;
    let bundle = Bundle::from_directory(
        &root.join(&job.version),
        "aio-plugin.toml",
        git.into(),
        job.source_revision,
        job.version,
    )?;
    build_request(host, &format!("/jobs/{}/package", job.id))?
        .header("x-aio-build-lease", &job.lease)
        .header("content-type", "application/vnd.aio.component+gzip")
        .body(bundle.encode()?)
        .send()
        .await?
        .error_for_status()?;
    build_request(host, &format!("/jobs/{}/complete", job.id))?
        .json(&BuildReport {
            lease: job.lease,
            error: None,
            documentation: Documentation::default(),
        })
        .send()
        .await?
        .error_for_status()?;
    let delivery = host.state.service::<dyn DeliveryService>()?;
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            let builds = delivery.builds(git).await?;
            let state = &builds[0]["state"];
            anyhow::ensure!(state != "failed", "发布失败: {builds:?}");
            if state == "active" {
                return anyhow::Ok(());
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await??;
    Ok(bundle)
}
#[tokio::test]
#[ignore = "需要独立 PostgreSQL 和 AIO_DELIVERY_TOKEN"]
async fn delivery_publication_continues_without_rollout_and_rollout_can_be_enabled_online()
-> Result<()> {
    let root = tempfile::tempdir()?;
    let database = database().await?;
    let tenant = uuid::Uuid::new_v4().to_string();
    let git = format!(
        "https://git.example.net/delivery/{}.git",
        uuid::Uuid::new_v4()
    );
    let mut selected = plugins(Default::default(), true);
    selected.extend([
        Arc::new(PostgresDeliveryPlugin) as DynPlugin<RuntimeAssembly>,
        Arc::new(PostgresDocumentationPlugin),
        Arc::new(DeliveryPlugin),
    ]);
    let selected = identity_support::with_identity(Arc::new(Identity(tenant.clone())), selected);
    let mut config = super::super::fixture::config(root.path(), &database, "http://localhost");
    config.delivery = Some(DeliveryConfig {
        owner: "unused".into(),
        discovery_interval_seconds: 1,
        revision_interval_seconds: 1,
    });
    let mut host = Host::start_configured(config, selected.clone()).await?;
    let first = submit(&host, root.path(), &git, 'a', 1).await?;
    let catalog = host.state.service::<dyn ComponentCatalog>()?;
    let (source, _) = catalog.published(&git, None).await?.context("发布缺失")?;
    success(host.install(&first).await?).await?;
    host.assert_call(source, 1, &first).await?;
    let second = submit(&host, root.path(), &git, 'b', 2).await?;
    assert_eq!(
        catalog
            .published(&git, None)
            .await?
            .context("发布缺失")?
            .1
            .digest,
        second.digest
    );
    host.assert_call(source, 1, &first).await?;
    // 安装自动升级插件后执行既有租户选择；不重新提交或上传构建。
    let mut with_rollout = selected.clone();
    with_rollout.push(Arc::new(PostgresDeliveryRolloutPlugin));
    host.reconfigure(with_rollout).await?;
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let entry = host
                .state
                .service::<dyn ComponentCatalog>()?
                .installation(&tenant, source)
                .await?
                .context("安装缺失")?;
            if entry
                .installation
                .is_some_and(|i| i.digest == second.digest)
            {
                return anyhow::Ok(());
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await??;
    host.assert_call(source, 2, &second).await?;
    host.reconfigure(selected).await?;
    let third = submit(&host, root.path(), &git, 'c', 1).await?;
    assert_ne!(third.digest, second.digest);
    host.assert_call(source, 2, &second).await?;
    let pool = host.state.service::<sqlx::PgPool>()?;
    sqlx::query("DELETE FROM delivery_jobs WHERE git=$1")
        .bind(&git)
        .execute(pool.as_ref())
        .await?;
    sqlx::query("DELETE FROM delivery_sources WHERE git=$1")
        .bind(&git)
        .execute(pool.as_ref())
        .await?;
    host.stop().await
}
