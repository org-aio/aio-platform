use anyhow::{Context as _, Result, ensure};
use az_plugin_package::PluginPackage;
use sqlx::{PgPool, Row};

use super::{RuntimeState, repository::is_package_revision, store::PluginStore};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS plugin_packages (
    revision TEXT PRIMARY KEY CHECK (revision ~ '^[0-9a-f]{64}$'),
    git TEXT NOT NULL,
    version TEXT NOT NULL,
    source_revision TEXT,
    manifest_toml TEXT NOT NULL,
    artifact_sha256 TEXT NOT NULL,
    archive BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(git, version)
);
"#;

pub(super) async fn migrate(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql(SCHEMA)
        .execute(pool)
        .await
        .context("创建二进制插件包表失败")?;
    Ok(())
}

impl PluginStore {
    pub(super) async fn save_package(&self, package: &PluginPackage) -> Result<()> {
        let archive_package = package.clone();
        let archive = tokio::task::spawn_blocking(move || archive_package.encode())
            .await
            .context("等待插件包编码失败")??;
        let mut transaction = self.pool.begin().await?;
        // 全局配额和版本唯一性在同一事务中串行校验，避免并发上传越过限制。
        sqlx::query("SELECT pg_advisory_xact_lock(1835626038, 1)")
            .execute(&mut *transaction)
            .await?;
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT revision FROM plugin_packages WHERE git = $1 AND version = $2",
        )
        .bind(&package.git)
        .bind(&package.version)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(revision) = existing {
            ensure!(
                revision == package.rev,
                "同一来源的版本号已发布其他内容，请使用新的版本号"
            );
            transaction.commit().await?;
            return Ok(());
        }
        let usage = sqlx::query(
            "SELECT COUNT(*)::BIGINT AS packages, COALESCE(SUM(octet_length(archive)), 0)::BIGINT AS bytes FROM plugin_packages",
        ).fetch_one(&mut *transaction).await?;
        let packages: i64 = usage.try_get("packages")?;
        let bytes: i64 = usage.try_get("bytes")?;
        ensure!(
            packages < quota("AIO_PLUGIN_PACKAGE_MAX_REVISIONS", 1024),
            "插件中心版本数量超过宿主配额"
        );
        ensure!(
            bytes.saturating_add(i64::try_from(archive.len())?)
                <= quota("AIO_PLUGIN_PACKAGE_MAX_BYTES", 2 * 1024 * 1024 * 1024),
            "插件中心二进制存储超过宿主配额"
        );
        sqlx::query(
            "INSERT INTO plugin_packages (revision, git, version, source_revision, manifest_toml, artifact_sha256, archive) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        ).bind(&package.rev).bind(&package.git).bind(&package.version).bind(&package.source_revision)
            .bind(&package.manifest_toml).bind(&package.artifact_sha256).bind(&archive)
            .execute(&mut *transaction).await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(super) async fn package_archive(&self, revision: &str) -> Result<Option<Vec<u8>>> {
        ensure!(
            is_package_revision(revision),
            "插件包版本必须是完整 SHA-256"
        );
        sqlx::query_scalar("SELECT archive FROM plugin_packages WHERE revision = $1")
            .bind(revision)
            .fetch_optional(&self.pool)
            .await
            .map_err(Into::into)
    }

    pub(super) async fn downloadable_package(&self, revision: &str) -> Result<Option<Vec<u8>>> {
        ensure!(
            is_package_revision(revision),
            "插件包版本必须是完整 SHA-256"
        );
        sqlx::query_scalar(
            "SELECT p.archive FROM plugin_packages p WHERE p.revision = $1 AND EXISTS (SELECT 1 FROM plugin_revisions r JOIN plugin_sources s ON s.id = r.source_id WHERE r.revision = p.revision AND s.git = p.git)",
        ).bind(revision).fetch_optional(&self.pool).await.map_err(Into::into)
    }

    pub(super) async fn published_package_entry(
        &self,
        git: &str,
        revision: &str,
    ) -> Result<Option<crate::runtime::MarketplaceEntry>> {
        let manifest: Option<String> = sqlx::query_scalar(
            "SELECT p.manifest_toml FROM plugin_packages p WHERE p.git = $1 AND p.revision = $2 AND EXISTS (SELECT 1 FROM plugin_revisions r JOIN plugin_sources s ON s.id = r.source_id WHERE r.revision = p.revision AND s.git = p.git)",
        ).bind(git).bind(revision).fetch_optional(&self.pool).await?;
        let Some(manifest) = manifest else {
            return Ok(None);
        };
        let manifest = az_plugin_manifest::parse_manifest(&manifest)?;
        let metadata = manifest
            .plugin
            .marketplace
            .context("已发布插件包缺少市场元数据")?;
        Ok(Some(crate::runtime::MarketplaceEntry {
            parent_git: None,
            parent_title: None,
            git: git.to_owned(),
            rev: revision.to_owned(),
            title: metadata.title,
            summary: metadata.summary,
            license: metadata.license,
            tags: metadata.tags,
            installed: false,
            source_id: None,
            state: None,
            active_revision: None,
            runtime: manifest.plugin.runtime.map(|runtime| runtime.kind),
            capabilities: manifest.plugin.capabilities,
        }))
    }
}

impl RuntimeState {
    pub(super) async fn restore_package_cache(&self, revision: &str) -> Result<()> {
        if !is_package_revision(revision) {
            return Ok(());
        }
        let archive = self
            .store
            .package_archive(revision)
            .await?
            .context("插件中心没有保存该内容版本的二进制包")?;
        let package = tokio::task::spawn_blocking(move || PluginPackage::decode(&archive))
            .await
            .context("等待数据库插件包验证失败")??;
        ensure!(
            package.rev == revision,
            "数据库插件包与请求的内容版本不一致"
        );
        self.repository.stage_publish(&package).await?;
        Ok(())
    }
}

fn quota(name: &str, default: i64) -> i64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

#[cfg(test)]
#[path = "package_store_tests.rs"]
mod tests;
