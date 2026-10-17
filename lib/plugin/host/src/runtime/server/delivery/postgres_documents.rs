use super::PluginDocumentation;
use anyhow::Result;
use az_plugin_delivery::{DocumentImage, Documentation};
use sqlx::{PgPool, Row};
use std::sync::Arc;

/// 正式 PostgreSQL 文档实现；发布可见性与版本列表仍在同一目录中查询。
#[dill::component]
#[dill::interface(dyn PluginDocumentation)]
#[dill::scope(dill::Singleton)]
pub(crate) struct PostgresDocumentation {
    pool: Arc<PgPool>,
}
#[async_trait::async_trait]
impl PluginDocumentation for PostgresDocumentation {
    async fn save(&self, revision: &str, doc: &Documentation) -> Result<()> {
        super::documents::validate_documentation(doc)?;
        sqlx::query("INSERT INTO plugin_documents(revision,readme,images) VALUES($1,$2,$3) ON CONFLICT(revision) DO NOTHING")
        .bind(revision).bind(&doc.readme).bind(serde_json::to_value(&doc.images)?).execute(self.pool.as_ref()).await?;
        Ok(())
    }
    async fn details(&self, revision: &str) -> Result<Option<serde_json::Value>> {
        let package = sqlx::query("SELECT p.git,p.version,p.source_revision,p.created_at::TEXT,d.readme FROM plugin_packages p LEFT JOIN plugin_documents d ON d.revision=p.revision WHERE p.revision=$1 AND EXISTS(SELECT 1 FROM plugin_revisions r WHERE r.revision=p.revision)")
        .bind(revision).fetch_optional(self.pool.as_ref()).await?;
        let Some(package) = package else {
            return Ok(None);
        };
        let git: String = package.try_get("git")?;
        let versions = sqlx::query("SELECT p.revision,p.version,p.source_revision,p.created_at::TEXT FROM plugin_packages p WHERE git=$1 AND EXISTS(SELECT 1 FROM plugin_revisions r WHERE r.revision=p.revision) ORDER BY p.created_at DESC LIMIT 20").bind(&git).fetch_all(self.pool.as_ref()).await?;
        let versions = versions.into_iter().map(|r| Ok::<_,sqlx::Error>(serde_json::json!({"revision":r.try_get::<String,_>("revision")?,"version":r.try_get::<String,_>("version")?,"source_revision":r.try_get::<Option<String>,_>("source_revision")?,"created_at":r.try_get::<String,_>("created_at")?}))).collect::<Result<Vec<_>,_>>()?;
        Ok(Some(serde_json::json!({
            "readme":package.try_get::<Option<String>,_>("readme")?.unwrap_or_default(),
            "version":package.try_get::<String,_>("version")?, "source_revision":package.try_get::<Option<String>,_>("source_revision")?,
            "git":git,"versions":versions
        })))
    }
    async fn image(&self, revision: &str, path: &str) -> Result<Option<DocumentImage>> {
        let value: Option<serde_json::Value> = sqlx::query_scalar("SELECT d.images->$2 FROM plugin_documents d WHERE d.revision=$1 AND EXISTS(SELECT 1 FROM plugin_revisions r WHERE r.revision=d.revision)").bind(revision).bind(path).fetch_optional(self.pool.as_ref()).await?.flatten();
        value
            .map(serde_json::from_value)
            .transpose()
            .map_err(Into::into)
    }
}
