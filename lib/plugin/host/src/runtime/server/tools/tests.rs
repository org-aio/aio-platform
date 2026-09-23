use anyhow::Result;
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::str::FromStr;

#[tokio::test]
#[ignore = "需要隔离 PostgreSQL，设置 AIO_TEST_DATABASE_URL"]
async fn cli_versions_are_persistent_immutable_and_share_the_marketplace_shape() -> Result<()> {
    let url = std::env::var("AIO_TEST_DATABASE_URL")?;
    let admin = PgPool::connect(&url).await?;
    let schema = format!("cli_market_{}", uuid::Uuid::new_v4().simple());
    sqlx::raw_sql(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await?;
    let options = PgConnectOptions::from_str(&url)?.options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new().connect_with(options).await?;
    let result = exercise(&pool).await;
    pool.close().await;
    sqlx::raw_sql(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await?;
    result
}

async fn exercise(pool: &PgPool) -> Result<()> {
    super::migrate(pool).await?;
    super::migrate(pool).await?;
    let original = super::storage::get(pool, "codex-model-sync", "0.4.1")
        .await?
        .unwrap();
    assert!(
        super::storage::get(pool, "codex-model-sync", "9.9.9")
            .await?
            .is_none()
    );
    let mut next = original.clone();
    next.version = "0.4.10".into();
    sqlx::query("INSERT INTO marketplace_tools(id,version,manifest) VALUES($1,$2,$3)")
        .bind(&next.id)
        .bind(&next.version)
        .bind(serde_json::to_value(&next)?)
        .execute(pool)
        .await?;
    let entries = super::entries(pool).await?;
    let values = serde_json::to_value(entries)?;
    assert_eq!(values.as_array().unwrap().len(), 1);
    assert_eq!(values[0]["rev"], "0.4.10");
    assert_eq!(values[0]["cli"]["id"], "codex-model-sync");
    assert_eq!(values[0]["installed"], false);
    assert!(
        values[0].get("device_installed").is_none(),
        "CLI 市场列表不能暴露或聚合跨设备安装状态"
    );
    assert!(
        values[0]["tags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tag| tag == "cli")
    );
    assert_eq!(
        super::storage::get(pool, "codex-model-sync", "0.4.1").await?,
        Some(original)
    );
    super::storage::remove(pool, "codex-model-sync").await?;
    super::migrate(pool).await?;
    assert!(super::entries(pool).await?.is_empty());
    assert!(
        super::storage::get(pool, "codex-model-sync", "0.4.1")
            .await?
            .is_none()
    );
    let retained: i64 =
        sqlx::query_scalar("SELECT count(*) FROM marketplace_tools WHERE id='codex-model-sync'")
            .fetch_one(pool)
            .await?;
    assert_eq!(retained, 2);
    Ok(())
}
