use anyhow::{Result, ensure};
use az_plugin_bundle::Bundle;
use sqlx::{PgPool, Row, postgres::PgRow};

use super::{ComponentCatalogEntry, ComponentInstallation, ComponentSource, ComponentVersion};

pub(super) fn entry(row: PgRow) -> Result<ComponentCatalogEntry> {
    let tenant: Option<String> = row.try_get("tenant_id")?;
    let installation = tenant
        .map(|tenant| {
            Ok::<_, anyhow::Error>(ComponentInstallation {
                tenant,
                digest: row.try_get("installed_digest")?,
                generation: row.try_get("generation")?,
                enabled: row.try_get("enabled")?,
                excluded_digest: row.try_get("excluded_digest")?,
            })
        })
        .transpose()?;
    Ok(ComponentCatalogEntry {
        source: ComponentSource {
            id: row.try_get("source_id")?,
            git: row.try_get("git")?,
            parent: row.try_get("parent_git")?,
        },
        menu_source: row.try_get("menu_source")?,
        version: ComponentVersion {
            digest: row.try_get("digest")?,
            description: serde_json::from_value(row.try_get("description")?)?,
            metadata: serde_json::from_value(row.try_get("metadata")?)?,
            capabilities: serde_json::from_value(row.try_get("capabilities")?)?,
            permissions: row.try_get("permissions")?,
            required_permissions: row.try_get("required_permissions")?,
        },
        installation,
    })
}

// 旧记录的权限索引只能从不可变整包回填，不能以空集合代替解析或摘要错误。
pub(super) async fn backfill(pool: &PgPool) -> Result<()> {
    let digests = sqlx::query_scalar::<_, String>(
        "SELECT digest FROM component_versions WHERE permissions IS NULL OR required_permissions IS NULL",
    ).fetch_all(pool).await?;
    for digest in digests {
        let archive: Vec<u8> =
            sqlx::query_scalar("SELECT archive FROM component_versions WHERE digest=$1")
                .bind(&digest)
                .fetch_one(pool)
                .await?;
        let bundle = Bundle::decode(&archive)?.verify()?;
        ensure!(bundle.digest() == digest, "权限索引与整包摘要不匹配");
        sqlx::query("UPDATE component_versions SET permissions=$2,required_permissions=$3 WHERE digest=$1 AND (permissions IS NULL OR required_permissions IS NULL)")
            .bind(&digest).bind(&bundle.manifest().plugin.permissions).bind(&bundle.manifest().plugin.required_permissions).execute(pool).await?;
    }
    Ok(())
}
