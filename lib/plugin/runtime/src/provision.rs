use std::{str::FromStr, time::Duration};

use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use sqlparser::{
    ast::{
        AlterColumnOperation, AlterTableOperation, ColumnOption, DropBehavior, ObjectName,
        Statement, TableConstraint,
    },
    dialect::PostgreSqlDialect,
    parser::Parser,
};
use sqlx::{
    ConnectOptions, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};

use crate::ScopedDatabase;

pub struct DatabaseProvisioner {
    pub(crate) pool: PgPool,
    pub(crate) connection: PgConnectOptions,
}

impl DatabaseProvisioner {
    pub async fn connect(url: &str) -> Result<Self> {
        let connection = PgConnectOptions::from_str(url)
            .context("数据库地址无效")?
            .disable_statement_logging();
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(3))
            .connect_with(connection.clone())
            .await?;
        let public_access: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_namespace, LATERAL aclexplode(coalesce(nspacl, acldefault('n', nspowner))) a WHERE nspname = 'public' AND grantee = 0)",
        ).fetch_one(&pool).await?;
        ensure!(
            !public_access,
            "运行库必须先撤销 public schema 的 PUBLIC 权限"
        );
        Ok(Self { pool, connection })
    }

    pub async fn create(
        &self,
        source: &str,
        tenant: &str,
        initial_schema: &str,
    ) -> Result<ScopedDatabase> {
        let schema = namespace(source, tenant);
        let names = role_namespace(&self.connection, source, tenant);
        let role = format!("r_{names}");
        let owner = format!("o_{names}");
        let mut random = [0u8; 32];
        getrandom::fill(&mut random)
            .map_err(|error| anyhow::anyhow!("生成数据库凭据失败: {error}"))?;
        let password = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let statements = Parser::parse_sql(&PostgreSqlDialect {}, initial_schema)?;
        ensure!(
            !statements.is_empty() && statements.len() <= 64,
            "初始化迁移数量无效"
        );
        for statement in &statements {
            validate_migration(statement)?;
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET LOCAL statement_timeout = '5s'")
            .execute(&mut *tx)
            .await?;
        // 名称只由摘要生成，密码只含十六进制字符，不接受插件提供的 SQL 标识符。
        for sql in [
            format!(
                "CREATE ROLE {role} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS CONNECTION LIMIT 4 PASSWORD '{password}'"
            ),
            format!(
                "CREATE ROLE {owner} NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS"
            ),
            format!("CREATE SCHEMA {schema} AUTHORIZATION {owner}"),
            format!("GRANT USAGE ON SCHEMA {schema} TO {role}"),
            format!("ALTER ROLE {role} SET search_path TO {schema}"),
            format!("ALTER ROLE {role} SET statement_timeout TO '3s'"),
            format!("ALTER ROLE {role} SET idle_in_transaction_session_timeout TO '5s'"),
            format!("ALTER ROLE {role} SET work_mem TO '4MB'"),
            format!("ALTER ROLE {role} SET temp_file_limit TO '16MB'"),
            format!("SET LOCAL search_path TO {schema}"),
            format!("SET LOCAL ROLE {owner}"),
        ] {
            sqlx::query(&sql).execute(&mut *tx).await?;
        }
        for statement in statements {
            sqlx::query(&statement.to_string())
                .execute(&mut *tx)
                .await
                .context("初始化插件 schema 失败")?;
        }
        for sql in [
            format!(
                "GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA {schema} TO {role}"
            ),
            format!("GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA {schema} TO {role}"),
        ] {
            sqlx::query(&sql).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        ScopedDatabase::connect_options(
            self.connection.clone().username(&role).password(&password),
            &role,
            &schema,
            source,
            tenant,
        )
        .await
    }
}

pub(crate) fn role_namespace(connection: &PgConnectOptions, source: &str, tenant: &str) -> String {
    // PostgreSQL 角色属于实例级对象；同一插件在不同数据库中必须使用不同凭据。
    let database = connection
        .get_database()
        .unwrap_or(connection.get_username());
    namespace(&format!("{}:{database}{source}", database.len()), tenant)
}

pub(crate) fn validate_role(role: &str) -> Result<()> {
    ensure!(
        role.len() == 44
            && role.starts_with("r_p_")
            && role[4..].bytes().all(|b| b.is_ascii_hexdigit()),
        "持久数据库角色无效"
    );
    Ok(())
}

pub(crate) fn namespace(source: &str, tenant: &str) -> String {
    let digest = Sha256::digest(format!("{}:{source}{tenant}", source.len()).as_bytes());
    format!("p_{digest:x}")[..42].to_owned()
}

pub(crate) fn validate_migration(statement: &Statement) -> Result<()> {
    match statement {
        Statement::CreateTable(table) => ensure!(
            table.name.0.len() == 1
                && table.query.is_none()
                && table.like.is_none()
                && table.clone.is_none(),
            "初始化表不能引用其他 schema 或复制已有表"
        ),
        Statement::CreateIndex(index) => {
            ensure!(index.table_name.0.len() == 1, "索引不能引用其他 schema")
        }
        Statement::AlterTable(table) => {
            ensure!(
                table.name.0.len() == 1
                    && table.location.is_none()
                    && table.on_cluster.is_none()
                    && table.table_type.is_none(),
                "表迁移不能引用其他 schema 或外部位置"
            );
            for operation in &table.operations {
                ensure!(
                    matches!(
                        operation,
                        AlterTableOperation::AddColumn { .. }
                            | AlterTableOperation::AddConstraint { .. }
                            | AlterTableOperation::AlterColumn {
                                op: AlterColumnOperation::DropNotNull,
                                ..
                            }
                            | AlterTableOperation::DropConstraint {
                                drop_behavior: None | Some(DropBehavior::Restrict),
                                ..
                            }
                    ),
                    "表迁移只允许新增列及非级联的约束修订"
                );
                match operation {
                    AlterTableOperation::AddConstraint {
                        constraint: TableConstraint::ForeignKey(key),
                        ..
                    } => validate_reference(&key.foreign_table)?,
                    AlterTableOperation::AddColumn { column_def, .. } => {
                        for option in &column_def.options {
                            if let ColumnOption::ForeignKey(key) = &option.option {
                                validate_reference(&key.foreign_table)?;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => anyhow::bail!("受控迁移只接受建表、建索引、新增列及约束修订"),
    }
    crate::database_query::validate_nodes(statement)
}

fn validate_reference(name: &ObjectName) -> Result<()> {
    let value = name.to_string().to_ascii_lowercase();
    ensure!(
        name.0.len() == 1
            && !value.trim_matches('"').starts_with("pg_")
            && value.trim_matches('"') != "information_schema",
        "外键不能引用其他 schema 或系统表"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed(sql: &str) -> bool {
        Parser::parse_sql(&PostgreSqlDialect {}, sql).is_ok_and(|statements| {
            statements
                .iter()
                .all(|statement| validate_migration(statement).is_ok())
        })
    }

    #[test]
    fn allows_scoped_additions_and_constraint_revisions() {
        for sql in [
            "CREATE TABLE intake (id UUID PRIMARY KEY, available_at TIMESTAMPTZ NOT NULL DEFAULT now())",
            "ALTER TABLE sources ADD COLUMN route TEXT",
            "ALTER TABLE sources ADD CONSTRAINT state_check CHECK (state IN ('pending','recorded'))",
            "ALTER TABLE sources DROP CONSTRAINT state_check",
            "ALTER TABLE sources DROP CONSTRAINT state_check RESTRICT",
            "ALTER TABLE sources ALTER COLUMN provider_id DROP NOT NULL",
        ] {
            assert!(allowed(sql), "{sql}");
        }
    }

    #[test]
    fn rejects_external_destructive_and_privileged_alterations() {
        for sql in [
            "ALTER TABLE other.sources ADD COLUMN x TEXT",
            "ALTER TABLE pg_authid ADD COLUMN x TEXT",
            "ALTER TABLE sources DROP COLUMN ciphertext",
            "ALTER TABLE sources DROP CONSTRAINT state_check CASCADE",
            "ALTER TABLE sources OWNER TO admin",
            "ALTER TABLE sources SET SCHEMA other",
            "ALTER TABLE sources DISABLE ROW LEVEL SECURITY",
            "ALTER TABLE sources ADD COLUMN x TEXT DEFAULT set_config('role','admin',false)",
            "ALTER TABLE sources ADD CONSTRAINT foreign_id FOREIGN KEY(id) REFERENCES other.sources(id)",
            "ALTER TABLE sources ADD COLUMN parent TEXT REFERENCES other.sources(id)",
            "ALTER TABLE sources ADD COLUMN parent TEXT REFERENCES pg_authid(rolname)",
            "ALTER TABLE sources ADD COLUMN safe TEXT, DROP COLUMN ciphertext",
        ] {
            assert!(!allowed(sql), "{sql}");
        }
    }
}
