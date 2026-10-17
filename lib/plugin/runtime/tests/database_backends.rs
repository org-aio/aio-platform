use anyhow::Result;
use az_plugin_runtime::{DatabaseValue, InvocationResources};

#[path = "database_backend/policy.rs"]
mod policy;
#[cfg(feature = "wasm")]
#[path = "database_backend/releases.rs"]
mod releases;
#[cfg(feature = "wasm")]
#[path = "database_backend/slots.rs"]
mod slots;
#[path = "database_backend/sqlite.rs"]
mod sqlite;

#[tokio::test]
async fn sqlite_transactions_preserve_scope_rollback_and_failure_boundaries() -> Result<()> {
    let root = tempfile::tempdir()?;
    let database =
        sqlite::SqliteDatabase::open(&root.path().join("first.db"), "source", "first").await?;
    let other =
        sqlite::SqliteDatabase::open(&root.path().join("second.db"), "source", "second").await?;
    let resources = InvocationResources {
        database: Some(database.clone()),
        ..Default::default()
    };
    resources.validate_scope("source", "first")?;
    assert!(resources.validate_scope("source", "second").is_err());
    assert!(resources.validate_scope("other", "first").is_err());
    assert!(database.process_connection().is_err());
    let mut transaction = database.begin().await?;
    transaction
        .execute(
            "INSERT INTO records VALUES(1,$1)",
            vec![DatabaseValue::Text("committed".into())],
        )
        .await?;
    transaction.commit().await?;
    let mut transaction = database.begin().await?;
    transaction
        .execute("UPDATE records SET value='discarded'", vec![])
        .await?;
    drop(transaction);
    let mut transaction = database.begin().await?;
    assert_eq!(
        transaction
            .query("SELECT value FROM records", vec![])
            .await?
            .values,
        vec![vec![DatabaseValue::Text("committed".into())]]
    );
    transaction
        .execute("UPDATE records SET value='failed'", vec![])
        .await?;
    assert!(
        transaction
            .query("SELECT * FROM missing", vec![])
            .await
            .is_err()
    );
    assert!(transaction.commit().await.is_err());
    let mut transaction = database.begin().await?;
    assert_eq!(
        transaction
            .query("SELECT value FROM records", vec![])
            .await?
            .values,
        vec![vec![DatabaseValue::Text("committed".into())]]
    );
    transaction.rollback().await?;
    let mut isolated = other.begin().await?;
    assert!(
        isolated
            .query("SELECT * FROM records", vec![])
            .await?
            .values
            .is_empty()
    );
    isolated.rollback().await?;
    Ok(())
}
