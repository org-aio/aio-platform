//! 默认目录的跨连接原子提交验收。
use super::*;

#[tokio::test]
#[ignore = "需要独立本机 PostgreSQL"]
async fn postgres_compare_and_swap_rejects_concurrent_install_and_stale_removal() -> Result<()> {
    let database = database().await?;
    let root = tempfile::tempdir()?;
    let tenant = Uuid::new_v4().to_string();
    let bundle = package(
        root.path(),
        &format!("https://github.com/example/cas-{tenant}.git"),
        1,
        "1.0.0",
    )?;
    let host = Host::start(root.path(), &database, &tenant, Default::default()).await?;
    let source: Uuid = success(host.publish(&bundle).await?).await?["data"]["source_id"]
        .as_str()
        .context("缺少来源")?
        .parse()?;
    let first = host.state.service::<dyn ComponentCatalog>()?;
    let pool = sqlx::PgPool::connect(&database).await?;
    let second = PostgresComponentCatalog::open(pool.clone()).await?;
    let commit = || ComponentInstallationCommit {
        tenant: tenant.clone(),
        source,
        digest: bundle.digest.clone(),
        excluded_digest: None,
        expected_generation: None,
    };
    let (a, b) = tokio::join!(first.install(commit()), second.install(commit()));
    assert_ne!(a.is_ok(), b.is_ok(), "并发首次安装只能成功一次");
    assert!(first.initialized(&tenant, &bundle.git).await?);
    let generation = first
        .installation(&tenant, source)
        .await?
        .context("缺少安装")?
        .installation
        .context("缺少安装记录")?
        .generation;
    first
        .install(ComponentInstallationCommit {
            expected_generation: Some(generation),
            ..commit()
        })
        .await?;
    assert!(
        second
            .remove(&tenant, source, generation, true)
            .await
            .is_err()
    );
    assert!(
        second
            .install(ComponentInstallationCommit {
                expected_generation: Some(generation),
                ..commit()
            })
            .await
            .is_err()
    );
    assert!(second.installation("other", source).await?.is_none());
    let (key_a, key_b) = tokio::join!(
        first.process_key(source, &tenant, vec![1]),
        second.process_key(source, &tenant, vec![2])
    );
    assert_eq!(key_a?, key_b?);
    let current = first
        .installation(&tenant, source)
        .await?
        .context("缺少安装")?
        .installation
        .context("缺少安装记录")?;
    first
        .remove(&tenant, source, current.generation, true)
        .await?;
    host.stop().await?;
    pool.close().await;
    Ok(())
}
