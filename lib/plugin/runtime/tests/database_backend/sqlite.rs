use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use az_plugin_runtime::{
    DatabaseBackend, DatabaseRows, DatabaseTransaction, DatabaseValue, ScopedDatabase,
};
use futures_util::TryStreamExt;
use sqlx::{
    Column, Row, Sqlite, SqlitePool, Transaction, TypeInfo, ValueRef,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

// 仅用于验证可替换性的真实 SQLite 后端，不作为生产数据库插件发布。
pub struct SqliteDatabase {
    pub pool: SqlitePool,
}

pub async fn pool(path: &Path) -> Result<SqlitePool> {
    Ok(SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(true)
                .busy_timeout(Duration::from_secs(2)),
        )
        .await?)
}

impl SqliteDatabase {
    pub async fn open(path: &Path, source: &str, tenant: &str) -> Result<ScopedDatabase> {
        let pool = pool(path).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS records(id INTEGER PRIMARY KEY,value TEXT)")
            .execute(&pool)
            .await?;
        ScopedDatabase::new(Arc::new(Self { pool }), source, tenant)
    }
}

#[async_trait::async_trait]
impl DatabaseBackend for SqliteDatabase {
    fn validate_migrations(&self, migrations: &[(String, String)]) -> Result<()> {
        ensure!(
            migrations
                .iter()
                .all(|(_, sql)| sql == "CREATE TABLE records(id INTEGER PRIMARY KEY,value TEXT)"),
            "夹具只预装一项固定迁移"
        );
        Ok(())
    }
    async fn begin(&self) -> Result<Box<dyn DatabaseTransaction>> {
        Ok(Box::new(SqliteTransaction(self.pool.begin().await?)))
    }
}

struct SqliteTransaction(Transaction<'static, Sqlite>);

fn bind(
    statement: &str,
    parameters: Vec<DatabaseValue>,
) -> Result<sqlx::query::Query<'_, Sqlite, sqlx::sqlite::SqliteArguments<'_>>> {
    // 数据库文件已按租户分开；夹具只允许参数化 DML，不能 ATTACH 或改变事务。
    let command = statement.split_whitespace().next().context("缺少 SQL")?;
    ensure!(
        matches!(command, "SELECT" | "INSERT" | "UPDATE" | "DELETE") && !statement.contains(';'),
        "夹具拒绝结构或事务命令"
    );
    let mut query = sqlx::query(statement);
    for value in parameters {
        query = match value {
            DatabaseValue::Null => query.bind(None::<String>),
            DatabaseValue::Boolean(value) => query.bind(value),
            DatabaseValue::Integer(value) => query.bind(value),
            DatabaseValue::Real(value) => query.bind(value),
            DatabaseValue::Text(value) | DatabaseValue::Json(value) => query.bind(value),
            DatabaseValue::Bytes(value) => query.bind(value),
        };
    }
    Ok(query)
}

#[async_trait::async_trait]
impl DatabaseTransaction for SqliteTransaction {
    async fn query(
        &mut self,
        statement: &str,
        parameters: Vec<DatabaseValue>,
    ) -> Result<DatabaseRows> {
        let mut stream = bind(statement, parameters)?.fetch(&mut *self.0);
        let mut output = DatabaseRows {
            columns: vec![],
            values: vec![],
        };
        while let Some(row) = stream.try_next().await? {
            ensure!(output.values.len() < 1000, "夹具查询行数超限");
            output.columns = row
                .columns()
                .iter()
                .map(|column| column.name().into())
                .collect();
            let mut values = Vec::new();
            for (index, column) in row.columns().iter().enumerate() {
                values.push(if row.try_get_raw(index)?.is_null() {
                    DatabaseValue::Null
                } else {
                    match column.type_info().name() {
                        "INTEGER" => DatabaseValue::Integer(row.try_get(index)?),
                        "REAL" => DatabaseValue::Real(row.try_get(index)?),
                        "BLOB" => DatabaseValue::Bytes(row.try_get(index)?),
                        _ => DatabaseValue::Text(row.try_get(index)?),
                    }
                });
            }
            output.values.push(values);
        }
        Ok(output)
    }
    async fn execute(&mut self, statement: &str, parameters: Vec<DatabaseValue>) -> Result<u64> {
        Ok(bind(statement, parameters)?
            .execute(&mut *self.0)
            .await?
            .rows_affected())
    }
    async fn commit(self: Box<Self>) -> Result<()> {
        self.0.commit().await.context("提交 SQLite 夹具失败")
    }
    async fn rollback(self: Box<Self>) -> Result<()> {
        self.0.rollback().await.context("回滚 SQLite 夹具失败")
    }
}
