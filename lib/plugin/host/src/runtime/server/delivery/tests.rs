use anyhow::Result;
use az_plugin_delivery::Documentation;
use sqlx::postgres::PgPoolOptions;

use super::{documents, store};

#[tokio::test]
#[ignore = "需要独立本机 AIO_DELIVERY_TEST_DATABASE_URL"]
async fn persists_leases_rejects_obsolete_builds_and_preserves_document_versions() -> Result<()> {
    let address = std::env::var("AIO_DELIVERY_TEST_DATABASE_URL")?;
    let url = reqwest::Url::parse(&address)?;
    anyhow::ensure!(
        url.host_str() == Some("127.0.0.1") && url.path() == "/aio_delivery_test",
        "仅允许独立本机交付测试数据库"
    );
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&address)
        .await?;
    let schema = format!("delivery_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&pool)
        .await?;
    sqlx::query(&format!("SET search_path TO {schema}"))
        .execute(&pool)
        .await?;
    let plugin_store = super::super::store::PluginStore::new(pool.clone());
    plugin_store.migrate().await?;
    let git = "https://github.com/example/plugin.git";
    let first = "a".repeat(40);
    let next = "b".repeat(40);
    sqlx::query("INSERT INTO delivery_sources(git,branch,desired_sha) VALUES($1,'main',$2)")
        .bind(git)
        .bind(&first)
        .execute(&pool)
        .await?;
    sqlx::query("INSERT INTO delivery_jobs(git,source_revision,recipe) VALUES($1,$2,$3)")
        .bind(git)
        .bind(&first)
        .bind(serde_json::json!({"environment":"typescript","command":["sh","scripts/build.sh"]}))
        .execute(&pool)
        .await?;
    let job = store::claim(&pool).await?.expect("必须领取任务");
    assert!(store::claim(&pool).await?.is_none(), "有效租约不得重复领取");
    assert!(store::check_lease(&pool, job.id, "wrong").await.is_err());
    sqlx::query("UPDATE delivery_jobs SET lease_until=now()-interval '1 second' WHERE id=$1")
        .bind(job.id)
        .execute(&pool)
        .await?;
    let resumed = store::claim(&pool).await?.expect("过期任务必须恢复");
    assert_eq!(resumed.version, job.version);
    assert_ne!(resumed.lease, job.lease);
    assert!(store::check_lease(&pool, job.id, &job.lease).await.is_err());
    sqlx::query("UPDATE delivery_sources SET desired_sha=$2 WHERE git=$1")
        .bind(git)
        .bind(&next)
        .execute(&pool)
        .await?;
    assert!(
        store::check_lease(&pool, job.id, &resumed.lease)
            .await
            .is_err(),
        "较新提交必须使旧构建失效"
    );
    let package = az_plugin_package::PluginPackage::new(git.into(),"0.0.0-dev.1".into(),Some(first),"[plugin.runtime]\nkind='page-definition'\nartifact='pages.json'\n[plugin.marketplace]\ntitle='Test'\nsummary='Test'\nlicense='MIT'\ntags=['test']\n".into(),b"[]",Default::default())?;
    plugin_store.save_package(&package).await?;
    documents::save(
        &pool,
        &package.rev,
        &Documentation {
            readme: "# First".into(),
            images: Default::default(),
        },
    )
    .await?;
    documents::save(
        &pool,
        &package.rev,
        &Documentation {
            readme: "# Changed".into(),
            images: Default::default(),
        },
    )
    .await?;
    let stored: String =
        sqlx::query_scalar("SELECT readme FROM plugin_documents WHERE revision=$1")
            .bind(&package.rev)
            .fetch_one(&pool)
            .await?;
    assert_eq!(stored, "# First", "已保存的版本文档不能被覆盖");
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&pool)
        .await?;
    Ok(())
}
