use anyhow::{Result, ensure};
use az_tool::{ToolManifest, registration::Documentation};
use sqlx::PgPool;

pub(super) async fn publish(
    pool: &PgPool,
    manifest: &ToolManifest,
    doc: &Documentation,
    revision: &str,
    integrity: &str,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("cli-publication:{}", manifest.id))
        .execute(&mut *tx)
        .await?;
    let old: Vec<serde_json::Value> =
        sqlx::query_scalar("SELECT manifest FROM marketplace_tools WHERE id=$1")
            .bind(&manifest.id)
            .fetch_all(&mut *tx)
            .await?;
    let mut latest = true;
    for value in old {
        let existing: ToolManifest = serde_json::from_value(value)?;
        ensure!(
            existing.homepage.trim_end_matches(".git") == manifest.homepage,
            "工具 ID 已绑定其他来源仓库"
        );
        if existing.version == manifest.version {
            let publication: Option<(String, String)> = sqlx::query_as("SELECT source_revision,integrity FROM marketplace_tool_publications WHERE id=$1 AND version=$2").bind(&manifest.id).bind(&manifest.version).fetch_optional(&mut *tx).await?;
            ensure!(
                publication
                    .as_ref()
                    .is_some_and(|(sha, digest)| sha == revision && digest == integrity)
                    && existing == *manifest,
                "同一版本已登记不同内容"
            );
            tx.commit().await?;
            return Ok(());
        }
        latest &=
            semver::Version::parse(&manifest.version)? > semver::Version::parse(&existing.version)?;
    }
    sqlx::query("INSERT INTO marketplace_tools(id,version,manifest) VALUES($1,$2,$3)")
        .bind(&manifest.id)
        .bind(&manifest.version)
        .bind(serde_json::to_value(manifest)?)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO marketplace_tool_publications(id,version,git,source_revision,integrity) VALUES($1,$2,$3,$4,$5)").bind(&manifest.id).bind(&manifest.version).bind(&manifest.homepage).bind(revision).bind(integrity).execute(&mut *tx).await?;
    if latest {
        sqlx::query("INSERT INTO marketplace_tool_details(id,document) VALUES($1,$2) ON CONFLICT(id) DO UPDATE SET document=EXCLUDED.document").bind(&manifest.id).bind(serde_json::to_value(doc)?).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}
