use std::time::Duration;

use anyhow::{Context, Result, ensure};
use sqlx::{
    PgPool, Postgres, Transaction,
    postgres::{PgConnectOptions, PgPoolOptions},
};

#[derive(Clone)]
pub struct ScopedDatabase {
    pool: PgPool,
    source: String,
    tenant: String,
}

impl ScopedDatabase {
    /// 原生数据库实现使用此入口创建绑定；连接后检查实际角色和 schema 隔离。
    pub async fn connect_options(
        options: PgConnectOptions,
        role: &str,
        schema: &str,
        source: &str,
        tenant: &str,
    ) -> Result<Self> {
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
        Ok(Self {
            pool,
            source: source.into(),
            tenant: tenant.into(),
        })
    }

    pub(crate) fn matches(&self, source: &str, tenant: &str) -> bool {
        self.source == source && self.tenant == tenant
    }

    /// 为 Sidecar 导出已校验连接的相同凭据，不另设可绕过隔离检查的凭据通道。
    pub fn connection_options(&self) -> PgConnectOptions {
        self.pool.connect_options().as_ref().clone()
    }

    /// 供可信原生执行器开启受租户隔离和数据库超时限制的事务。
    pub async fn begin(&self) -> Result<Transaction<'static, Postgres>> {
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
        Ok(transaction)
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
            ScopedDatabase::connect_options(options, &role, "public", "source", "tenant")
                .await
                .is_err()
        );
        let provisioner = crate::DatabaseProvisioner::connect(&url).await?;
        let source = uuid::Uuid::new_v4().to_string();
        let database = provisioner
            .create(&source, "tenant", "CREATE TABLE records(id BIGINT)")
            .await?;
        let schema: String = sqlx::query_scalar("SELECT current_schema()")
            .fetch_one(&database.pool)
            .await?;
        let options = database.connection_options();
        let role = options.get_username().to_owned();
        // SET ROLE 不能把管理员登录伪装成专属角色，否则进程可以 RESET ROLE 提权。
        let masked = PgConnectOptions::from_str(&url)?
            .options([("role", role.as_str()), ("search_path", schema.as_str())]);
        assert!(
            ScopedDatabase::connect_options(masked, &role, &schema, &source, "tenant")
                .await
                .is_err()
        );
        assert!(
            ScopedDatabase::connect_options(options.clone(), &role, "public", &source, "tenant")
                .await
                .is_err()
        );
        sqlx::query(&format!("GRANT CREATE ON SCHEMA {schema} TO {role}"))
            .execute(&provisioner.pool)
            .await?;
        assert!(
            ScopedDatabase::connect_options(options.clone(), &role, &schema, &source, "tenant")
                .await
                .is_err()
        );
        sqlx::query(&format!("REVOKE CREATE ON SCHEMA {schema} FROM {role}"))
            .execute(&provisioner.pool)
            .await?;
        let reopened =
            ScopedDatabase::connect_options(options, &role, &schema, &source, "tenant").await?;
        let mut tx = reopened.begin().await?;
        sqlx::query("INSERT INTO records VALUES (1)")
            .execute(&mut *tx)
            .await?;
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
            .fetch_one(&first.pool)
            .await?;
        assert!(
            sqlx::query(&format!("SELECT * FROM {schema}.records"))
                .fetch_all(&second.pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("CREATE TABLE unauthorized(id BIGINT)")
                .execute(&first.pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("SET ROLE aio_test_admin")
                .execute(&first.pool)
                .await
                .is_err()
        );
        Ok(())
    }
}
