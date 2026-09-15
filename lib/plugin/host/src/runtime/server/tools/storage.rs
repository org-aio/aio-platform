use super::model::MarketplaceItem;
use anyhow::{Context as _, Result, ensure};
use az_tool::ToolManifest;
use sqlx::PgPool;

pub(in crate::runtime::server) async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql("CREATE TABLE IF NOT EXISTS marketplace_tools (id TEXT NOT NULL, version TEXT NOT NULL, manifest JSONB NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(), PRIMARY KEY(id, version))").execute(pool).await?;
    import(
        pool,
        include_str!("../../../../../../../tools/registry/codex-model-sync-0.1.4.json"),
    )
    .await?;
    if let Some(directory) = std::env::var_os("AIO_TOOL_REGISTRY_DIR") {
        for entry in std::fs::read_dir(directory).context("读取 CLI 市场导入目录失败")? {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                ensure!(
                    std::fs::metadata(&path)?.len() <= az_tool::MAX_MANIFEST_BYTES,
                    "CLI 描述过大"
                );
                import(pool, &std::fs::read_to_string(path)?).await?;
            }
        }
    }
    Ok(())
}

async fn import(pool: &PgPool, text: &str) -> Result<()> {
    let manifest: ToolManifest = serde_json::from_str(text)?;
    manifest.validate()?;
    sqlx::query("INSERT INTO marketplace_tools(id,version,manifest) VALUES($1,$2,$3) ON CONFLICT(id,version) DO NOTHING")
        .bind(&manifest.id).bind(&manifest.version).bind(serde_json::to_value(&manifest)?).execute(pool).await?;
    Ok(())
}

pub(in crate::runtime::server) async fn get(
    pool: &PgPool,
    id: &str,
    version: &str,
) -> Result<Option<ToolManifest>> {
    let value: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT manifest FROM marketplace_tools WHERE id=$1 AND version=$2")
            .bind(id)
            .bind(version)
            .fetch_optional(pool)
            .await?;
    value.map(decode).transpose()
}

fn decode(value: serde_json::Value) -> Result<ToolManifest> {
    let manifest: ToolManifest = serde_json::from_value(value)?;
    manifest.validate()?;
    Ok(manifest)
}

pub(in crate::runtime::server) async fn entries(pool: &PgPool) -> Result<Vec<MarketplaceItem>> {
    let rows: Vec<serde_json::Value> =
        sqlx::query_scalar("SELECT manifest FROM marketplace_tools ORDER BY id, created_at")
            .fetch_all(pool)
            .await?;
    let mut latest = std::collections::BTreeMap::<String, ToolManifest>::new();
    for row in rows {
        let manifest = decode(row)?;
        let replace = latest
            .get(&manifest.id)
            .map(|old| {
                Ok::<_, anyhow::Error>(
                    semver::Version::parse(&manifest.version)?
                        > semver::Version::parse(&old.version)?,
                )
            })
            .transpose()?
            .unwrap_or(true);
        if replace {
            latest.insert(manifest.id.clone(), manifest);
        }
    }
    Ok(latest.into_values().map(Into::into).collect())
}
