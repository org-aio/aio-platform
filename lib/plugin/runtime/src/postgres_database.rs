use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use sqlx::{
    ConnectOptions, PgPool, Postgres, Transaction,
    postgres::{PgConnectOptions, PgPoolOptions},
};

use crate::{
    DatabaseBackend, DatabaseEndpoint, DatabaseRows, DatabaseTransaction, DatabaseValue,
    ProcessDatabaseConnection, ScopedDatabase,
};

/// 默认 PostgreSQL 专属角色连接；创建时核验登录、schema 和角色权限。
pub struct PostgresDatabase {
    pool: PgPool,
}

impl PostgresDatabase {
    /// 原生数据库实现使用此入口创建绑定；连接后检查实际角色和 schema 隔离。
    pub async fn connect(
        options: PgConnectOptions,
        role: &str,
        schema: &str,
        source: &str,
        tenant: &str,
    ) -> Result<ScopedDatabase> {
        ensure!(
            !source.is_empty() && !tenant.is_empty() && tenant.len() <= 128,
            "数据库归属无效"
        );
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(3))
            .connect_with(options)
            .await
            .context("连接插件专属数据库失败")?;
        let safe: bool = sqlx::query_scalar(
            "SELECT current_user = $1 AND session_user = $1 AND current_schema() = $2 AND NOT (rolsuper OR rolcreaterole OR rolcreatedb OR rolbypassrls) AND NOT has_schema_privilege(current_user, $2, 'CREATE') FROM pg_roles WHERE rolname = current_user",
        )
        .bind(role)
        .bind(schema)
        .fetch_one(&pool)
        .await?;
        ensure!(safe, "插件数据库角色或 schema 不符合隔离要求");
        let foreign_schemas: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_namespace WHERE nspname <> $1 AND nspname NOT LIKE 'pg_%' AND nspname <> 'information_schema' AND has_schema_privilege(current_user, oid, 'USAGE')",
        )
        .bind(schema)
        .fetch_one(&pool)
        .await?;
        ensure!(foreign_schemas == 0, "插件角色可以访问其他业务 schema");
        let memberships: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_auth_members WHERE member = (SELECT oid FROM pg_roles WHERE rolname = current_user)",
        )
        .fetch_one(&pool)
        .await?;
        ensure!(memberships == 0, "插件角色不能继承或切换到其他角色");
        ScopedDatabase::new(Arc::new(Self { pool }), source, tenant)
    }
}

#[async_trait::async_trait]
impl DatabaseBackend for PostgresDatabase {
    fn validate_migrations(&self, migrations: &[(String, String)]) -> Result<()> {
        use sqlparser::{dialect::PostgreSqlDialect, parser::Parser};
        for (_, sql) in migrations {
            let statements = Parser::parse_sql(&PostgreSqlDialect {}, sql)?;
            ensure!(
                !statements.is_empty() && statements.len() <= 64,
                "迁移语句数无效"
            );
            for statement in statements {
                crate::provision::validate_migration(&statement)?;
            }
        }
        Ok(())
    }

    fn process_connection(&self) -> Result<ProcessDatabaseConnection> {
        let connection = self.pool.connect_options();
        let port = connection.get_port();
        let upstream = match connection.get_socket() {
            Some(path) => DatabaseEndpoint::Unix(path.join(format!(".s.PGSQL.{port}"))),
            None => DatabaseEndpoint::Tcp {
                host: connection.get_host().into(),
                port,
            },
        };
        let direct_url = connection.to_url_lossy();
        let mut isolated_url = direct_url.clone();
        isolated_url.set_host(Some("localhost"))?;
        isolated_url
            .set_port(Some(5432))
            .map_err(|_| anyhow::anyhow!("数据库端口无效"))?;
        // 移除原连接的 Unix socket 参数，隔离进程只能访问映射后的目录。
        let parameters: Vec<_> = isolated_url
            .query_pairs()
            .filter(|(name, _)| name != "host" && name != "port")
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
            .collect();
        isolated_url.set_query(None);
        isolated_url
            .query_pairs_mut()
            .extend_pairs(parameters)
            .append_pair("host", "/database");
        Ok(ProcessDatabaseConnection {
            direct_url: direct_url.into(),
            isolated_url: isolated_url.into(),
            socket_name: ".s.PGSQL.5432".into(),
            upstream,
        })
    }

    /// 供可信原生执行器开启受租户隔离和数据库超时限制的事务。
    async fn begin(&self) -> Result<Box<dyn DatabaseTransaction>> {
        let mut transaction = self.pool.begin().await.context("开始插件事务失败")?;
        sqlx::query("SET LOCAL statement_timeout = '3s'")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("SET LOCAL lock_timeout = '1s'")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("SET LOCAL idle_in_transaction_session_timeout = '5s'")
            .execute(&mut *transaction)
            .await?;
        Ok(Box::new(PostgresTransaction(transaction)))
    }
}

struct PostgresTransaction(Transaction<'static, Postgres>);

#[async_trait::async_trait]
impl DatabaseTransaction for PostgresTransaction {
    async fn query(
        &mut self,
        statement: &str,
        parameters: Vec<DatabaseValue>,
    ) -> Result<DatabaseRows> {
        crate::database_query::query(&mut self.0, statement, parameters).await
    }
    async fn execute(&mut self, statement: &str, parameters: Vec<DatabaseValue>) -> Result<u64> {
        crate::database_query::execute(&mut self.0, statement, parameters).await
    }
    async fn commit(self: Box<Self>) -> Result<()> {
        self.0.commit().await.context("提交插件事务失败")
    }
    async fn rollback(self: Box<Self>) -> Result<()> {
        self.0.rollback().await.context("回滚插件事务失败")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "需要独立本机 PostgreSQL"]
    async fn external_bindings_reject_admin_wrong_schema_and_schema_creation() -> Result<()> {
        use std::str::FromStr;
        let url = std::env::var("AIO_TEST_DATABASE_URL")?;
        let options = PgConnectOptions::from_str(&url)?;
        let role = options.get_username().to_owned();
        assert!(
            PostgresDatabase::connect(options, &role, "public", "source", "tenant")
                .await
                .is_err()
        );
        let provisioner = crate::DatabaseProvisioner::connect(&url).await?;
        let source = uuid::Uuid::new_v4().to_string();
        let database = provisioner
            .create(&source, "tenant", "CREATE TABLE records(id BIGINT)")
            .await?;
        let schema: String = sqlx::query_scalar("SELECT current_schema()")
            .fetch_one(&PgPool::connect(&database.process_connection()?.direct_url).await?)
            .await?;
        let options = PgConnectOptions::from_str(&database.process_connection()?.direct_url)?;
        let role = options.get_username().to_owned();
        // SET ROLE 不能把管理员登录伪装成专属角色，否则进程可以 RESET ROLE 提权。
        let masked = PgConnectOptions::from_str(&url)?
            .options([("role", role.as_str()), ("search_path", schema.as_str())]);
        assert!(
            PostgresDatabase::connect(masked, &role, &schema, &source, "tenant")
                .await
                .is_err()
        );
        assert!(
            PostgresDatabase::connect(options.clone(), &role, "public", &source, "tenant")
                .await
                .is_err()
        );
        sqlx::query(&format!("GRANT CREATE ON SCHEMA {schema} TO {role}"))
            .execute(&provisioner.pool)
            .await?;
        assert!(
            PostgresDatabase::connect(options.clone(), &role, &schema, &source, "tenant")
                .await
                .is_err()
        );
        sqlx::query(&format!("REVOKE CREATE ON SCHEMA {schema} FROM {role}"))
            .execute(&provisioner.pool)
            .await?;
        let reopened =
            PostgresDatabase::connect(options, &role, &schema, &source, "tenant").await?;
        let mut tx = reopened.begin().await?;
        tx.execute("INSERT INTO records VALUES (1)", vec![]).await?;
        tx.commit().await?;
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires a disposable PostgreSQL database"]
    async fn database_roles_deny_other_plugin_schemas_without_the_sql_filter() -> Result<()> {
        let provisioner =
            crate::DatabaseProvisioner::connect(&std::env::var("AIO_TEST_DATABASE_URL")?).await?;
        let source = format!(
            "role-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        );
        let first = provisioner
            .create(&source, "first", "CREATE TABLE records(id BIGINT)")
            .await?;
        let second = provisioner
            .create(&source, "second", "CREATE TABLE records(id BIGINT)")
            .await?;
        let schema: String = sqlx::query_scalar("SELECT current_schema()")
            .fetch_one(&PgPool::connect(&first.process_connection()?.direct_url).await?)
            .await?;
        assert!(
            sqlx::query(&format!("SELECT * FROM {schema}.records"))
                .fetch_all(&PgPool::connect(&second.process_connection()?.direct_url).await?)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("CREATE TABLE unauthorized(id BIGINT)")
                .execute(&PgPool::connect(&first.process_connection()?.direct_url).await?)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("SET ROLE aio_test_admin")
                .execute(&PgPool::connect(&first.process_connection()?.direct_url).await?)
                .await
                .is_err()
        );
        Ok(())
    }
}
