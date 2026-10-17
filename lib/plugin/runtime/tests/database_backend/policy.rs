use std::{sync::Arc, time::Duration};

use anyhow::Result;
use az_plugin_runtime::{
    DatabaseBackend, DatabaseRows, DatabaseTransaction, DatabaseValue, ScopedDatabase,
};

use super::sqlite::SqliteDatabase;

#[derive(Clone, Copy)]
enum Fault {
    Rows,
    Bytes,
    Columns,
    Wait,
}

struct FaultyBackend {
    database: SqliteDatabase,
    fault: Fault,
}
struct FaultyTransaction {
    transaction: Box<dyn DatabaseTransaction>,
    fault: Fault,
}

#[async_trait::async_trait]
impl DatabaseBackend for FaultyBackend {
    fn validate_migrations(&self, migrations: &[(String, String)]) -> Result<()> {
        self.database.validate_migrations(migrations)
    }
    async fn begin(&self) -> Result<Box<dyn DatabaseTransaction>> {
        Ok(Box::new(FaultyTransaction {
            transaction: self.database.begin().await?,
            fault: self.fault,
        }))
    }
}

#[async_trait::async_trait]
impl DatabaseTransaction for FaultyTransaction {
    async fn query(&mut self, _: &str, _: Vec<DatabaseValue>) -> Result<DatabaseRows> {
        let values = match self.fault {
            Fault::Rows => vec![vec![DatabaseValue::Null]; 1001],
            Fault::Bytes => vec![vec![DatabaseValue::Bytes(vec![0; 4 * 1024 * 1024 + 1])]],
            Fault::Columns => vec![vec![]],
            Fault::Wait => std::future::pending().await,
        };
        Ok(DatabaseRows {
            columns: vec!["value".into()],
            values,
        })
    }
    async fn execute(&mut self, statement: &str, parameters: Vec<DatabaseValue>) -> Result<u64> {
        self.transaction.execute(statement, parameters).await
    }
    async fn commit(self: Box<Self>) -> Result<()> {
        self.transaction.commit().await
    }
    async fn rollback(self: Box<Self>) -> Result<()> {
        self.transaction.rollback().await
    }
}

#[tokio::test]
async fn runtime_rejects_invalid_backend_results_and_rolls_back_real_writes() -> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("data.db");
    let safe = SqliteDatabase::open(&path, "source", "tenant").await?;
    for fault in [Fault::Rows, Fault::Bytes, Fault::Columns] {
        let database = ScopedDatabase::new(
            Arc::new(FaultyBackend {
                database: SqliteDatabase {
                    pool: super::sqlite::pool(&path).await?,
                },
                fault,
            }),
            "source",
            "tenant",
        )?;
        let mut transaction = database.begin().await?;
        transaction
            .execute("INSERT INTO records VALUES(1,'pending')", vec![])
            .await?;
        assert!(
            transaction
                .query("SELECT value FROM records", vec![])
                .await
                .is_err()
        );
        assert!(transaction.commit().await.is_err());
        let mut read = safe.begin().await?;
        assert!(
            read.query("SELECT * FROM records", vec![])
                .await?
                .values
                .is_empty()
        );
        read.rollback().await?;
    }
    Ok(())
}

#[tokio::test]
async fn cancelled_and_timed_out_queries_invalidate_the_transaction_and_release_writes()
-> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("data.db");
    let safe = SqliteDatabase::open(&path, "source", "tenant").await?;
    let database = ScopedDatabase::new(
        Arc::new(FaultyBackend {
            database: SqliteDatabase {
                pool: super::sqlite::pool(&path).await?,
            },
            fault: Fault::Wait,
        }),
        "source",
        "tenant",
    )?;
    for cancelled in [true, false] {
        let mut transaction = database.begin().await?;
        transaction
            .execute("INSERT INTO records VALUES(1,'pending')", vec![])
            .await?;
        if cancelled {
            assert!(
                tokio::time::timeout(
                    Duration::from_millis(20),
                    transaction.query("SELECT * FROM records", vec![])
                )
                .await
                .is_err()
            );
        } else {
            let error = transaction
                .query("SELECT * FROM records", vec![])
                .await
                .err()
                .ok_or_else(|| anyhow::anyhow!("应超时"))?;
            assert!(error.to_string().contains("超时"));
        }
        assert!(transaction.commit().await.is_err());
        let mut read = safe.begin().await?;
        assert!(
            read.query("SELECT * FROM records", vec![])
                .await?
                .values
                .is_empty()
        );
        read.rollback().await?;
    }
    Ok(())
}

#[tokio::test]
async fn oversized_request_closes_the_transaction_before_calling_backend() -> Result<()> {
    let root = tempfile::tempdir()?;
    let database = SqliteDatabase::open(&root.path().join("data.db"), "source", "tenant").await?;
    for (statement, parameters) in [
        ("SELECT 1".into(), vec![DatabaseValue::Null; 257]),
        (" ".repeat(65537), vec![]),
    ] {
        let mut transaction = database.begin().await?;
        transaction
            .execute("INSERT INTO records VALUES(1,'pending')", vec![])
            .await?;
        assert!(transaction.query(&statement, parameters).await.is_err());
        assert!(transaction.commit().await.is_err());
        let mut read = database.begin().await?;
        assert!(
            read.query("SELECT * FROM records", vec![])
                .await?
                .values
                .is_empty()
        );
        read.rollback().await?;
    }
    Ok(())
}
