use anyhow::{Context, Result, ensure};
use az_plugin_contract::{InvocationScope, RequestContext};
use sqlx::{Row, postgres::PgConnectOptions};

use crate::{
    DatabaseProvisioner, Keyring,
    provision::{namespace, validate_role},
};

impl DatabaseProvisioner {
    /// 仅由宿主为隔离进程提供已迁移的专属数据角色，不返回管理连接。
    pub async fn process_connection(
        &self,
        source: &str,
        tenant: &str,
        keyring: &Keyring,
    ) -> Result<PgConnectOptions> {
        let row = sqlx::query("SELECT schema_name,role_name,ciphertext FROM aio_plugin_host.database_bindings WHERE source_id=$1 AND tenant_id=$2")
            .bind(source).bind(tenant).fetch_one(&self.pool).await?;
        let schema = namespace(source, tenant);
        let role: String = row.get("role_name");
        validate_role(&role)?;
        ensure!(
            row.get::<String, _>("schema_name") == schema
                && row.get::<String, _>("role_name") == role,
            "数据库绑定不一致"
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
        let password = String::from_utf8(keyring.open(
            &scope,
            "database-role",
            &row.get::<Vec<u8>, _>("ciphertext"),
        )?)
        .context("数据角色凭据无效")?;
        Ok(self.connection.clone().username(&role).password(&password))
    }
}
