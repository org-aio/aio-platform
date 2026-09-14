use std::time::Duration;

use anyhow::{Context as _, Result};

use super::*;
use crate::runtime::CapabilityManifest;

fn published_entry(git: &str, revision: &str) -> MarketplaceEntry {
    MarketplaceEntry {
        parent_git: None,
        parent_title: None,
        git: git.to_owned(),
        rev: revision.to_owned(),
        title: "离线插件".to_owned(),
        summary: "数据库已发布插件".to_owned(),
        license: "MIT".to_owned(),
        tags: vec!["test".to_owned()],
        installed: false,
        source_id: None,
        state: None,
        active_revision: None,
        runtime: Some(PluginRuntime::PageDefinition),
        capabilities: CapabilityManifest::default(),
    }
}

#[tokio::test]
async fn published_install_candidate_uses_local_artifact_without_git() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let cache = temporary.path().join("cache");
    let revision = "a".repeat(40);
    let root = cache.join(&revision);
    tokio::fs::create_dir_all(root.join("dist")).await?;
    tokio::fs::write(
        root.join("aio-plugin.toml"),
        r#"
[plugin.runtime]
kind = "page-definition"
artifact = "dist/pages.json"

[plugin.marketplace]
title = "Offline Plugin"
summary = "Database published plugin"
license = "MIT"
tags = ["test"]

[[plugin.subplugins]]
id = "offline"
pages = ["offline-page"]
"#,
    )
    .await?;
    tokio::fs::write(
        root.join("dist/pages.json"),
        br#"[{"id":"offline-page","label":"Offline","icon":null,"scene":{"id":"community","label":"Community"},"required_permission":null,"body":{"kind":"text","title":"Offline","content":"cached"}}]"#,
    )
    .await?;
    let repository = RepositoryInstaller::new(cache);
    let git = "https://github.com/example/aio-plugin-offline.git";
    let request = InstallPluginRequest {
        git: git.to_owned(),
        rev: None,
    };
    let publication = published_entry(git, &revision);

    let (discovered, detail) = tokio::time::timeout(
        Duration::from_secs(1),
        resolve_install_candidate(&repository, &request, Some(&publication)),
    )
    .await
    .context("已发布插件安装不应等待 Git 网络")??;

    assert_eq!(discovered.revision, revision);
    assert_eq!(discovered.pages.len(), 1);
    assert!(detail.contains("数据库已发布 artifact"));
    Ok(())
}

#[tokio::test]
async fn missing_published_cache_fails_without_git_fallback() -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let repository = RepositoryInstaller::new(temporary.path().join("cache"));
    let revision = "b".repeat(40);
    let git = "https://github.com/example/aio-plugin-offline.git";
    let request = InstallPluginRequest {
        git: git.to_owned(),
        rev: Some(revision.clone()),
    };
    let publication = published_entry(git, &revision);

    let result = tokio::time::timeout(
        Duration::from_secs(1),
        resolve_install_candidate(&repository, &request, Some(&publication)),
    )
    .await
    .context("缓存缺失时不应等待 Git 网络")?;
    let Err(error) = result else {
        anyhow::bail!("缓存缺失必须直接失败");
    };

    assert!(error.to_string().contains("本地 artifact"));
    Ok(())
}
