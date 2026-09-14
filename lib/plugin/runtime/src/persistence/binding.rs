use anyhow::{Context, Result, ensure};
use az_plugin_contract::{InvocationScope, RequestContext};
use sha2::{Digest, Sha256};
use sqlparser::{dialect::PostgreSqlDialect, parser::Parser};
use sqlx::Row;

use crate::{
    DatabaseProvisioner, Keyring, ScopedDatabase,
    provision::{namespace, role_namespace, validate_migration, validate_role},
};

impl DatabaseProvisioner {
    pub async fn install(
        &self,
        source: &str,
        tenant: &str,
        migrations: &[(String, String)],
        keyring: &Keyring,
    ) -> Result<ScopedDatabase> {
        ensure!(
            !source.is_empty()
                && !tenant.is_empty()
                && !migrations.is_empty()
                && migrations.len() <= 64,
            "数据库绑定或迁移无效"
        );
        let scope = InvocationScope {
            source_id: source.into(),
            revision: String::new(),
            context: RequestContext {
                tenant_id: Some(tenant.into()),
                ..Default::default()
            },
            grants: Default::default(),
        };
        let schema = namespace(source, tenant);

        let mut parsed = Vec::new();
        let mut previous = "";
        for (name, sql) in migrations {
            ensure!(
                name.as_str() > previous && name.len() <= 160 && sql.len() <= 512_000,
                "迁移必须有序且名称唯一"
            );
            previous = name;
            let statements =
                Parser::parse_sql(&PostgreSqlDialect {}, sql).context("解析结构迁移失败")?;
            ensure!(
                !statements.is_empty() && statements.len() <= 64,
                "迁移语句数无效"
            );
            for statement in &statements {
                validate_migration(statement)?;
            }
            parsed.push((
                name,
                format!("{:x}", Sha256::digest(sql.as_bytes())),
                statements,
            ));
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('aio-plugin-bindings',0))")
            .execute(&mut *tx)
            .await?;
        sqlx::query("CREATE SCHEMA IF NOT EXISTS aio_plugin_host")
            .execute(&mut *tx)
            .await?;
        sqlx::query("REVOKE ALL ON SCHEMA aio_plugin_host FROM PUBLIC")
            .execute(&mut *tx)
            .await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS aio_plugin_host.database_bindings(source_id TEXT NOT NULL,tenant_id TEXT NOT NULL,schema_name TEXT NOT NULL,role_name TEXT NOT NULL,ciphertext BYTEA NOT NULL,migrations JSONB NOT NULL,PRIMARY KEY(source_id,tenant_id))").execute(&mut *tx).await?;
        let existing = sqlx::query("SELECT schema_name,role_name,ciphertext,migrations FROM aio_plugin_host.database_bindings WHERE source_id=$1 AND tenant_id=$2 FOR UPDATE").bind(source).bind(tenant).fetch_optional(&mut *tx).await?;
        let role = existing
            .as_ref()
            .map(|row| row.get::<String, _>("role_name"))
            .unwrap_or_else(|| format!("r_{}", role_namespace(&self.connection, source, tenant)));
        validate_role(&role)?;
        let owner = format!("o_{}", &role[2..]);
        let (password, applied) = if let Some(row) = existing {
            ensure!(
                row.get::<String, _>("schema_name") == schema
                    && row.get::<String, _>("role_name") == role,
                "持久数据库绑定不一致"
            );
            let password = String::from_utf8(keyring.open(
                &scope,
                "database-role",
                &row.get::<Vec<u8>, _>("ciphertext"),
            )?)
            .context("数据库凭据格式无效")?;
            let applied: Vec<(String, String)> = serde_json::from_value(row.get("migrations"))?;
            ensure!(
                applied.len() <= parsed.len()
                    && applied
                        .iter()
                        .zip(&parsed)
                        .all(|((name, hash), (current, digest, _))| name == *current
                            && hash == digest),
                "已应用迁移不能修改、删除或重排"
            );
            (password, applied.len())
        } else {
            let mut random = [0u8; 32];
            getrandom::fill(&mut random).map_err(|_| anyhow::anyhow!("随机凭据生成失败"))?;
            let password = random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            for statement in [
                format!(
                    "CREATE ROLE {role} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS CONNECTION LIMIT 6 PASSWORD '{password}'"
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
            ] {
                sqlx::query(&statement)
                    .execute(&mut *tx)
                    .await
                    .context("创建数据库隔离角色失败")?;
            }
            (password, 0)
        };
        sqlx::query(&format!("SET LOCAL search_path TO {schema}"))
            .execute(&mut *tx)
            .await?;
        sqlx::query(&format!("SET LOCAL ROLE {owner}"))
            .execute(&mut *tx)
            .await?;
        for (_, _, statements) in parsed.iter().skip(applied) {
            for statement in statements {
                sqlx::query(&statement.to_string())
                    .execute(&mut *tx)
                    .await
                    .context("应用结构迁移失败")?;
            }
        }
        sqlx::query(&format!(
            "GRANT SELECT,INSERT,UPDATE,DELETE ON ALL TABLES IN SCHEMA {schema} TO {role}"
        ))
        .execute(&mut *tx)
        .await?;
        sqlx::query(&format!(
            "GRANT USAGE,SELECT ON ALL SEQUENCES IN SCHEMA {schema} TO {role}"
        ))
        .execute(&mut *tx)
        .await?;
        sqlx::query("RESET ROLE").execute(&mut *tx).await?;
        let history: Vec<_> = parsed.iter().map(|(name, hash, _)| (*name, hash)).collect();
        sqlx::query("INSERT INTO aio_plugin_host.database_bindings(source_id,tenant_id,schema_name,role_name,ciphertext,migrations) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(source_id,tenant_id) DO UPDATE SET ciphertext=EXCLUDED.ciphertext,migrations=EXCLUDED.migrations")
            .bind(source).bind(tenant).bind(&schema).bind(&role).bind(keyring.seal(&scope,"database-role",password.as_bytes())?).bind(serde_json::to_value(history)?).execute(&mut *tx).await?;
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
