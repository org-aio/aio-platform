use anyhow::Result;
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::str::FromStr as _;

#[tokio::test]
#[ignore = "需要隔离 PostgreSQL，设置 AIO_TEST_DATABASE_URL"]
async fn archives_sources_and_preserves_official_packages_and_bindings() -> Result<()> {
    let url = std::env::var("AIO_TEST_DATABASE_URL")?;
    let admin = PgPool::connect(&url).await?;
    let schema = format!("market_test_{}", uuid::Uuid::new_v4().simple());
    sqlx::raw_sql(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await?;
    let options = PgConnectOptions::from_str(&url)?.options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await?;
    let result = exercise(&pool).await;
    pool.close().await;
    sqlx::raw_sql(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await?;
    result
}

async fn exercise(pool: &PgPool) -> Result<()> {
    let store = super::super::store::PluginStore::new(pool.clone());
    store.migrate().await?;
    sqlx::raw_sql(r#"
        CREATE TABLE plugin_registries (id TEXT PRIMARY KEY, tenant_id TEXT, source TEXT, enabled BOOLEAN);
        INSERT INTO plugin_registries VALUES ('legacy','tenant-a','https://external.example/index.json',true);
        CREATE TABLE marketplace_registry_syncs(source TEXT PRIMARY KEY, last_error TEXT);
        INSERT INTO marketplace_registry_syncs VALUES ('https://external.example/index.json',NULL);
        INSERT INTO marketplace_entries(source,git,rev,title,summary,license,tags,capabilities)
        VALUES ('aio://published','https://github.com/owner/official.git','official','官方插件','','MIT','[]','{}'),
        ('https://external.example/index.json','https://github.com/owner/legacy.git','old','旧插件','','MIT','[]','{}');
        INSERT INTO plugin_sources(id,git) VALUES ('installed','https://github.com/owner/legacy.git');
        INSERT INTO plugin_revisions(id,source_id,revision,runtime,manifest,pages)
        VALUES ('old-revision','installed','old','page-definition','{}','[]');
        INSERT INTO tenant_plugin_bindings(tenant_id,source_id,revision_id) VALUES ('tenant-a','installed','old-revision');
        CREATE TABLE plugin_business_data(id TEXT PRIMARY KEY, value TEXT);
        INSERT INTO plugin_business_data VALUES ('note','keep');
    "#).execute(pool).await?;

    // 先证明中途失败不会删除旧配置；随后重复完整迁移，证明归档不被覆盖。
    let mut transaction = pool.begin().await?;
    sqlx::raw_sql(include_str!("migration.sql"))
        .execute(&mut *transaction)
        .await?;
    transaction.rollback().await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM plugin_registries")
            .fetch_one(pool)
            .await?,
        1
    );
    store.migrate().await?;
    store.migrate().await?;
    let entries = store.marketplace_entries("aio://published").await?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].title, "官方插件");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM marketplace_entries")
            .fetch_one(pool)
            .await?,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM marketplace_source_archive")
            .fetch_one(pool)
            .await?,
        3
    );
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM tenant_plugin_bindings WHERE tenant_id='tenant-a' AND revision_id='old-revision'").fetch_one(pool).await?, 1);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT value FROM plugin_business_data WHERE id='note'")
            .fetch_one(pool)
            .await?,
        "keep"
    );
    assert!(!sqlx::query_scalar::<_, bool>("SELECT to_regclass('plugin_registries') IS NOT NULL OR to_regclass('marketplace_registry_syncs') IS NOT NULL").fetch_one(pool).await?);
    Ok(())
}
