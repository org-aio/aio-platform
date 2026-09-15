use super::{Components, services};
use anyhow::Result;
use az_plugin_bundle::Bundle;
use sqlx::PgPool;
use uuid::Uuid;

// 已发布包只迁移一次权限索引，鉴权请求不读取整包或依赖角色表。
pub(super) async fn backfill(pool: &PgPool) -> Result<()> {
    let digests = sqlx::query_scalar::<_, String>(
        "SELECT digest FROM component_versions WHERE permissions IS NULL",
    )
    .fetch_all(pool)
    .await?;
    for digest in digests {
        let archive: Vec<u8> =
            sqlx::query_scalar("SELECT archive FROM component_versions WHERE digest=$1")
                .bind(&digest)
                .fetch_one(pool)
                .await?;
        let bundle = Bundle::decode(&archive)?.verify()?;
        sqlx::query(
            "UPDATE component_versions SET permissions=$2 WHERE digest=$1 AND permissions IS NULL",
        )
        .bind(&digest)
        .bind(&bundle.manifest().plugin.permissions)
        .execute(pool)
        .await?;
    }
    Ok(())
}

impl Components {
    pub async fn permissions(&self, tenant: &str) -> Result<Vec<String>> {
        let rows = sqlx::query_as::<_, (Uuid, Vec<String>)>(
            "SELECT i.source_id,v.permissions FROM component_installations i JOIN component_versions v ON v.digest=i.digest WHERE i.tenant_id=$1 AND i.enabled ORDER BY i.source_id",
        )
        .bind(tenant)
        .fetch_all(&self.pool)
        .await?;
        let mut permissions = rows
            .into_iter()
            .flat_map(|(source, names)| {
                names
                    .into_iter()
                    .map(move |name| services::permission(source, &name))
            })
            .collect::<Vec<_>>();
        if tenant == "development" {
            for (source, instance) in self.development.read().await.iter() {
                permissions.extend(
                    instance
                        .bundle
                        .manifest()
                        .plugin
                        .permissions
                        .iter()
                        .map(|name| services::permission(source, name)),
                );
            }
        }
        Ok(permissions)
    }
}
