use super::model::MarketplaceItem;
use anyhow::{Context as _, Result, ensure};
use az_tool::{ToolManifest, registration::Documentation};
use sqlx::PgPool;

pub(in crate::runtime::server) async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql("CREATE TABLE IF NOT EXISTS marketplace_tools (id TEXT NOT NULL, version TEXT NOT NULL, manifest JSONB NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(), PRIMARY KEY(id, version))").execute(pool).await?;
    sqlx::raw_sql("CREATE TABLE IF NOT EXISTS marketplace_tool_details (id TEXT PRIMARY KEY, document JSONB NOT NULL)").execute(pool).await?;
    sqlx::raw_sql("CREATE TABLE IF NOT EXISTS marketplace_tool_publications (id TEXT NOT NULL, version TEXT NOT NULL, git TEXT NOT NULL, source_revision TEXT NOT NULL, integrity TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(), PRIMARY KEY(id,version), FOREIGN KEY(id,version) REFERENCES marketplace_tools(id,version))").execute(pool).await?;
    import(
        pool,
        include_str!("../../../../../../../tools/registry/codex-model-sync-0.4.1.json"),
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
    let mut manifest = value.map(decode).transpose()?;
    if let Some(manifest) = manifest.as_mut()
        && let Some(doc) = documentation(pool, id).await?
    {
        apply_metadata(manifest, &doc);
    }
    Ok(manifest)
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
    let docs: Vec<(String, serde_json::Value)> =
        sqlx::query_as("SELECT id,document FROM marketplace_tool_details")
            .fetch_all(pool)
            .await?;
    for (id, value) in docs {
        if let Some(manifest) = latest.get_mut(&id) {
            apply_metadata(manifest, &serde_json::from_value(value)?);
        }
    }
    Ok(latest.into_values().map(Into::into).collect())
}

fn apply_metadata(manifest: &mut ToolManifest, doc: &Documentation) {
    manifest.title = doc.metadata.title.clone();
    manifest.summary = doc.metadata.summary.clone();
    manifest.homepage = doc.metadata.git.clone();
}

pub(super) async fn register(
    pool: &PgPool,
    manifest: &ToolManifest,
    doc: &Documentation,
) -> Result<()> {
    manifest.validate()?;
    let mut tx = pool.begin().await?;
    let result = sqlx::query("INSERT INTO marketplace_tools(id,version,manifest) VALUES($1,$2,$3) ON CONFLICT(id,version) DO NOTHING")
        .bind(&manifest.id).bind(&manifest.version).bind(serde_json::to_value(manifest)?).execute(&mut *tx).await?;
    if result.rows_affected() > 0 {
        sqlx::query("INSERT INTO marketplace_tool_details(id,document) VALUES($1,$2) ON CONFLICT(id) DO NOTHING")
            .bind(&manifest.id).bind(serde_json::to_value(doc)?).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

pub(super) async fn exists(pool: &PgPool, id: &str) -> Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM marketplace_tools WHERE id=$1)")
            .bind(id)
            .fetch_one(pool)
            .await?,
    )
}

pub(super) async fn documentation(pool: &PgPool, id: &str) -> Result<Option<Documentation>> {
    let value: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT document FROM marketplace_tool_details WHERE id=$1")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    value
        .map(serde_json::from_value)
        .transpose()
        .map_err(Into::into)
}

pub(super) async fn save_documentation(pool: &PgPool, id: &str, doc: &Documentation) -> Result<()> {
    sqlx::query("INSERT INTO marketplace_tool_details(id,document) VALUES($1,$2) ON CONFLICT(id) DO UPDATE SET document=EXCLUDED.document")
        .bind(id).bind(serde_json::to_value(doc)?).execute(pool).await?;
    Ok(())
}
