//! 真实 PostgreSQL 验证多发现器竞争、撤销墓碑与来源/任务同事务。
use super::super::fixture::{database, plugins};
use super::*;
use az_plugin_host::runtime::server::delivery::*;
use sqlx::PgPool;

fn target(revision: char) -> Result<DeliveryTarget> {
    Ok(DeliveryTarget {
        branch: "main".into(),
        source_revision: revision.to_string().repeat(40),
        recipe: serde_json::from_value(json!({"environment":"typescript","command":["fixture"]}))?,
    })
}
#[tokio::test]
#[ignore = "需要独立 PostgreSQL"]
async fn delivery_sources_compare_generations_revoke_leases_and_commit_atomically() -> Result<()> {
    let root = tempfile::tempdir()?;
    let database = database().await?;
    let mut selected = plugins(Default::default(), true);
    selected.extend([
        Arc::new(PostgresDeliveryPlugin) as DynPlugin<RuntimeAssembly>,
        Arc::new(PostgresDocumentationPlugin),
    ]);
    let selected = identity_support::with_identity(Arc::new(Identity("default".into())), selected);
    let host = Host::start_selected(root.path(), &database, selected.clone()).await?;
    let sources = host.state.service::<dyn DeliverySources>()?;
    let delivery = host.state.service::<dyn DeliveryService>()?;
    let pool = host.state.service::<PgPool>()?;
    // 非 GitHub 来源无需修改内核或默认构建服务。
    let git = format!("https://git.example.net/team/{}.git", uuid::Uuid::new_v4());
    let absent = format!("{git}-absent");
    assert!(sources.update(&absent, None, None).await?);
    assert!(!sources.update(&absent, None, Some(target('a')?)).await?);
    assert!(
        !sources
            .get(&absent)
            .await?
            .context("首次撤销墓碑缺失")?
            .enabled
    );
    let mut invalid = target('a')?;
    invalid.source_revision = "short".into();
    assert!(sources.update(&git, None, Some(invalid)).await.is_err());
    assert!(sources.get(&git).await?.is_none());
    let (a, b) = tokio::try_join!(
        sources.update(&git, None, Some(target('a')?)),
        sources.update(&git, None, Some(target('b')?))
    )?;
    assert_ne!(a, b, "首次发现也只能有一个提交获胜");
    let first = sources.get(&git).await?.context("来源缺失")?;
    let initial = delivery.claim().await?.context("构建缺失")?;
    assert_eq!(initial.git, git);
    assert_eq!(initial.source_revision, first.source_revision);
    assert_eq!(delivery.builds(&git).await?.len(), 1);
    assert!(sources.update(&git, Some(first.generation), None).await?);
    let disabled = sources.get(&git).await?.context("墓碑缺失")?;
    assert!(!disabled.enabled);
    assert!(!delivery.heartbeat(initial.id, &initial.lease).await?);
    assert!(delivery.claim().await?.is_none());
    assert!(
        !sources
            .update(&git, Some(first.generation), Some(target('c')?))
            .await?
    );
    assert!(!sources.update(&git, None, Some(target('c')?)).await?);

    // 任务写入失败时，来源代数、目标和已有任务必须一起回滚。
    let trigger = format!("source_commit_{}", uuid::Uuid::new_v4().simple());
    sqlx::raw_sql(&format!("CREATE FUNCTION {trigger}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.git='{git}' AND NEW.source_revision='{}' THEN RAISE EXCEPTION 'fixture source failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER {trigger} BEFORE INSERT ON delivery_jobs FOR EACH ROW EXECUTE FUNCTION {trigger}();", "c".repeat(40))).execute(pool.as_ref()).await?;
    let failed = sources
        .update(&git, Some(disabled.generation), Some(target('c')?))
        .await;
    assert!(
        format!("{:#}", failed.err().context("应拒绝提交")?).contains("fixture source failure")
    );
    assert_eq!(
        sources.get(&git).await?.context("墓碑缺失")?.generation,
        disabled.generation
    );
    assert_eq!(delivery.builds(&git).await?.len(), 1);
    sqlx::raw_sql(&format!(
        "DROP TRIGGER {trigger} ON delivery_jobs; DROP FUNCTION {trigger}();"
    ))
    .execute(pool.as_ref())
    .await?;
    assert!(
        sources
            .update(&git, Some(disabled.generation), Some(target('c')?))
            .await?
    );
    let c = sources.get(&git).await?.context("来源缺失")?;
    let old = delivery.claim().await?.context("新构建缺失")?;
    // 同一目标重复通知不能新建任务或夺取有效租约。
    assert!(
        sources
            .update(&git, Some(c.generation), Some(target('c')?))
            .await?
    );
    assert!(delivery.heartbeat(old.id, &old.lease).await?);
    assert_eq!(delivery.builds(&git).await?.len(), 2);
    let c = sources.get(&git).await?.context("来源缺失")?;
    assert!(sources.update(&git, Some(c.generation), None).await?);
    host.stop().await?;
    assert!(
        sources.enabled().await.is_err(),
        "连接池关闭错误不能静默吞掉"
    );

    let host = Host::start_selected(root.path(), &database, selected).await?;
    let sources = host.state.service::<dyn DeliverySources>()?;
    let delivery = host.state.service::<dyn DeliveryService>()?;
    let restored = sources.get(&git).await?.context("重启丢失墓碑")?;
    assert!(!restored.enabled);
    assert!(
        !sources
            .update(&git, Some(c.generation), Some(target('d')?))
            .await?
    );
    // 被撤销的同一源码可以显式重新入队，旧租约仍不能恢复。
    assert!(
        sources
            .update(&git, Some(restored.generation), Some(target('c')?))
            .await?
    );
    let resumed = delivery.claim().await?.context("同一源码未重新入队")?;
    assert_eq!(resumed.id, old.id);
    assert_ne!(resumed.lease, old.lease);
    assert!(!delivery.heartbeat(old.id, &old.lease).await?);
    let pool = host.state.service::<PgPool>()?;
    sqlx::query("DELETE FROM delivery_jobs WHERE git=$1")
        .bind(&git)
        .execute(pool.as_ref())
        .await?;
    sqlx::query("DELETE FROM delivery_sources WHERE git=$1")
        .bind(&git)
        .execute(pool.as_ref())
        .await?;
    sqlx::query("DELETE FROM delivery_sources WHERE git=$1")
        .bind(&absent)
        .execute(pool.as_ref())
        .await?;
    host.stop().await
}
