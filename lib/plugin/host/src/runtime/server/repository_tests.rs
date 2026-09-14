use super::*;
use az_plugin_manifest::parse_manifest;
use base64::Engine as _;
use flate2::{Compression, write::GzEncoder};
use std::io::Write as _;

use super::super::publication_validation::validate_publish_payload;
use az_plugin_package::PluginPackage;
use sha2::{Digest as _, Sha256};

fn package_test_request(artifact: &[u8]) -> Result<PluginPackage> {
    PluginPackage::new(
        "https://github.com/example/plugin.git".to_owned(),
        "1.0.0".to_owned(),
        None,
        "[plugin.runtime]\nkind = 'page-definition'\nartifact = 'dist/pages.json'\n\n[plugin.marketplace]\ntitle = 'Pages'\nsummary = 'Pages'\nlicense = 'MIT'\ntags = ['test']\n".to_owned(),
        artifact,
        Default::default(),
    )
}

#[test]
fn validates_canonical_https_git_sources() {
    assert!(validate_git("https://github.com/example/aio-plugin.git").is_ok());
    assert!(validate_git("http://github.com/example/aio-plugin.git").is_err());
    assert!(validate_git("https://user:secret@example.com/aio-plugin.git").is_err());
    assert!(validate_git("https://example.com/aio-plugin.git?rev=main").is_err());
    assert!(validate_git("https://example.com/aio-plugin.git#main").is_err());
    assert!(validate_git("https://example.com/aio-plugin").is_err());
    assert!(validate_git("https://example.com:8443/aio-plugin.git").is_err());
    assert!(validate_git("https://127.0.0.1/aio-plugin.git").is_err());
    assert!(validate_git("https://localhost/aio-plugin.git").is_err());
}

#[test]
fn rejects_non_public_network_destinations() {
    assert!(is_public_ip("140.82.112.3".parse().expect("公网 IPv4")));
    assert!(is_public_ip(
        "2606:50c0:8000::154".parse().expect("公网 IPv6")
    ));
    assert!(!is_public_ip("10.0.0.1".parse().expect("私网 IPv4")));
    assert!(!is_public_ip("100.64.0.1".parse().expect("共享 IPv4")));
    assert!(!is_public_ip("::1".parse().expect("环回 IPv6")));
    assert!(!is_public_ip(
        "::ffff:127.0.0.1".parse().expect("映射 IPv6")
    ));
}

#[test]
fn rejects_archives_over_the_expanded_quota() -> Result<()> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut header = tar::Header::new_gnu();
    header.set_path("plugin/oversized.bin")?;
    header.set_size(MAX_ARCHIVE_EXPANDED_BYTES + 1);
    header.set_cksum();
    encoder.write_all(header.as_bytes())?;
    let content = encoder.finish()?;
    let temporary = tempfile::tempdir()?;

    let error = extract_archive(content, temporary.path()).expect_err("展开超配额必须被拒绝");

    assert!(error.to_string().contains("展开后超过"));
    Ok(())
}

#[test]
fn enforces_aggregate_cache_bytes_and_revision_count() -> Result<()> {
    let cache = tempfile::tempdir()?;
    let revision = cache.path().join("a".repeat(40));
    std::fs::create_dir_all(&revision)?;
    std::fs::write(revision.join("artifact.bin"), b"12345")?;

    validate_cache_quota(cache.path(), u64::MAX, 1, 2, false)?;
    assert!(validate_cache_quota(cache.path(), 4, 1, 2, false).is_err());
    assert!(validate_cache_quota(cache.path(), u64::MAX, 1, 2, true).is_err());
    assert!(validate_cache_quota(cache.path(), u64::MAX, 1, 1, false).is_err());
    Ok(())
}

#[test]
fn builds_github_archive_for_full_revision() -> Result<()> {
    let revision = "0123456789012345678901234567890123456789";
    let (archive, resolved) =
        github_archive("https://github.com/example/aio-plugin.git", Some(revision))?
            .context("应生成 GitHub 归档")?;

    assert_eq!(resolved, revision);
    assert_eq!(
        archive.as_str(),
        "https://codeload.github.com/example/aio-plugin/tar.gz/0123456789012345678901234567890123456789"
    );
    Ok(())
}

#[test]
fn keeps_git_for_non_github_or_symbolic_revision() -> Result<()> {
    assert!(
        github_archive(
            "https://git.example.com/plugin.git",
            Some("0".repeat(40).as_str())
        )?
        .is_none()
    );
    assert!(github_archive("https://github.com/example/plugin.git", Some("main"))?.is_none());
    Ok(())
}

#[test]
fn rejects_capabilities_not_granted_to_online_publications() -> Result<()> {
    let manifest = parse_manifest(
        "[plugin.runtime]\nkind = 'process'\nartifact = 'plugin.js'\ncontainer_image = 'node:22@sha256:6c74791e557ce11fc957704f6d4fe134a7bc8d6f5ca4403205b2966bd488f6b3'\nentrypoint = ['node', '{artifact}']\nhealth_check = '/health'\n\n[plugin.capabilities]\ndatabase = true\n",
    )?;

    let error =
        ensure_publish_capabilities(&manifest).expect_err("未授权的数据库能力必须在暂存前被拒绝");

    assert!(error.to_string().contains("未授予"));
    Ok(())
}

#[test]
fn published_source_identity_is_a_supervisor_safe_uuid() -> Result<()> {
    let source_id = published_source_id("https://github.com/example/plugin.git");

    uuid::Uuid::parse_str(&source_id).context("发布来源身份必须是 UUID")?;
    assert_eq!(
        source_id,
        published_source_id("https://github.com/example/plugin.git")
    );
    Ok(())
}

#[test]
fn accepts_binary_package_without_git_proof() -> Result<()> {
    let request = package_test_request(b"[]")?;

    validate_publish_payload(&request)?;
    Ok(())
}

#[test]
fn accepts_frontend_bundle_with_declared_isolated_assets() -> Result<()> {
    let original = package_test_request(b"[]")?;
    let package = PluginPackage::new(
        original.git,
        original.version,
        None,
        format!(
            "{}\n[plugin.frontend]\npath='dist/web'\n",
            original.manifest_toml
        ),
        b"[]",
        [("index.html".to_owned(), b"<!doctype html>".to_vec())]
            .into_iter()
            .collect(),
    )?;
    assert!(package.verify().is_ok());
    assert_eq!(validate_publish_payload(&package)?.frontend.len(), 1);
    Ok(())
}

#[tokio::test]
async fn stages_complete_frontend_bundle_and_rejects_corrupted_cache() -> Result<()> {
    let original = package_test_request(b"[]")?;
    let package = PluginPackage::new(
        original.git,
        original.version,
        None,
        format!(
            "{}\n[plugin.frontend]\npath='dist/web'\n",
            original.manifest_toml
        ),
        b"[]",
        [
            ("index.html".to_owned(), b"<!doctype html>".to_vec()),
            (
                "assets/app.js".to_owned(),
                b"export const count = 0;".to_vec(),
            ),
        ]
        .into_iter()
        .collect(),
    )?;
    let temporary = tempfile::tempdir()?;
    let repository = RepositoryInstaller::new(temporary.path().to_path_buf());
    repository.stage_publish(&package).await?;
    let asset = temporary
        .path()
        .join(&package.rev)
        .join("dist/web/assets/app.js");
    assert_eq!(tokio::fs::read(&asset).await?, b"export const count = 0;");
    repository.stage_publish(&package).await?;
    tokio::fs::write(&asset, b"modified").await?;
    assert!(repository.stage_publish(&package).await.is_err());
    Ok(())
}

#[test]
fn rejects_artifact_tampered_after_packaging() -> Result<()> {
    let mut request = package_test_request(b"[]")?;
    let forged = br#"[{"forged":true}]"#;
    request.artifact_base64 = base64::engine::general_purpose::STANDARD.encode(forged);
    request.artifact_sha256 = format!("{:x}", Sha256::digest(forged));

    let error = match validate_publish_payload(&request) {
        Ok(_) => panic!("内容摘要不一致的 artifact 必须被拒绝"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("内容版本"));
    Ok(())
}

#[tokio::test]
async fn checks_out_the_exact_revision_before_publication() -> Result<()> {
    let repository = tempfile::tempdir()?;
    std::fs::write(repository.path().join("artifact.bin"), b"committed")?;
    run_git(
        None,
        &[
            "init",
            "--quiet",
            repository.path().to_str().context("测试路径不是 Unicode")?,
        ],
    )
    .await?;
    run_git(
        Some(repository.path()),
        &["config", "user.email", "test@example.com"],
    )
    .await?;
    run_git(
        Some(repository.path()),
        &["config", "user.name", "AIO Test"],
    )
    .await?;
    run_git(Some(repository.path()), &["add", "artifact.bin"]).await?;
    run_git(
        Some(repository.path()),
        &["commit", "--quiet", "-m", "committed"],
    )
    .await?;
    let revision = git_output(repository.path(), &["rev-parse", "HEAD"]).await?;
    let checkout = tempfile::tempdir()?;
    let staging = checkout.path().join("checkout");
    let source = format!("file://{}", repository.path().display());

    let (root, resolved) =
        checkout_revision(&source, Some(&revision), staging.as_path(), None).await?;

    assert_eq!(resolved, revision);
    assert_eq!(std::fs::read(root.join("artifact.bin"))?, b"committed");
    Ok(())
}

#[tokio::test]
async fn rejects_a_hexadecimal_ref_that_points_to_another_commit() -> Result<()> {
    let repository = tempfile::tempdir()?;
    std::fs::write(repository.path().join("artifact.bin"), b"committed")?;
    run_git(
        None,
        &[
            "init",
            "--quiet",
            repository.path().to_str().context("测试路径不是 Unicode")?,
        ],
    )
    .await?;
    run_git(
        Some(repository.path()),
        &["config", "user.email", "test@example.com"],
    )
    .await?;
    run_git(
        Some(repository.path()),
        &["config", "user.name", "AIO Test"],
    )
    .await?;
    run_git(Some(repository.path()), &["add", "artifact.bin"]).await?;
    run_git(
        Some(repository.path()),
        &["commit", "--quiet", "-m", "committed"],
    )
    .await?;
    let forged_revision = "f".repeat(40);
    run_git(
        Some(repository.path()),
        &["branch", forged_revision.as_str()],
    )
    .await?;
    let checkout = tempfile::tempdir()?;
    let source = format!("file://{}", repository.path().display());

    checkout_revision(
        &source,
        Some(&forged_revision),
        &checkout.path().join("checkout"),
        None,
    )
    .await
    .expect_err("十六进制 ref 不能冒充完整提交 SHA");
    Ok(())
}

#[tokio::test]
async fn publishes_validated_navigation_tree_artifact() -> Result<()> {
    let artifact = br#"{"id":"community","label":"Community","children":[{"id":"reports","label":"Reports","children":[{"id":"published-page","label":"Published","icon":"box","required_permission":"reports.read","body":{"kind":"text","title":"Published","content":"from CI"}}]}]}"#;
    let request = PluginPackage::new(
        "https://github.com/example/aio-plugin-published.git".to_owned(),
        "1.0.0".to_owned(),
        None,
        r#"
[plugin.runtime]
kind = "page-definition"
artifact = "dist/pages.json"

[plugin.marketplace]
title = "Published Pages"
summary = "Published page test"
license = "MIT"
tags = ["test"]

[plugin.capabilities]
network = []
filesystem = []
database = false

[[plugin.subplugins]]
id = "published"
pages = ["published-page"]
"#
        .to_owned(),
        artifact,
        Default::default(),
    )?;
    let temporary = tempfile::tempdir()?;
    let installer = RepositoryInstaller::new(temporary.path().join("cache"));

    let published = installer.publish(&request).await?;

    assert_eq!(published.runtime, PluginRuntime::PageDefinition);
    assert_eq!(published.pages.len(), 1);
    assert_eq!(published.pages[0].menu_path[0].id, "reports");
    assert_eq!(
        published.pages[0].required_permission.as_deref(),
        Some("reports.read")
    );
    assert_eq!(
        std::fs::read(installer.artifact(&request.rev, "dist/pages.json")?)?,
        artifact
    );
    let restored = installer
        .validate_published(&request.git, &request.rev)
        .await?;
    assert_eq!(restored.pages, published.pages);
    assert!(
        installer
            .artifact(&request.rev, "dist/pages.json")?
            .is_file()
    );
    Ok(())
}

#[tokio::test]
async fn publishes_validated_process_artifact() -> Result<()> {
    let artifact = b"committed process artifact";
    let request = PluginPackage::new(
        "https://github.com/example/aio-plugin-process.git".to_owned(),
        "1.0.0".to_owned(),
        None,
        r#"
[plugin.runtime]
kind = "process"
artifact = "dist/plugin.jar"
host_version = ">=2026.9.9"
container_image = "eclipse-temurin:21-jre@sha256:5c67d24ee8e3dd810b2a0cb6c3827ced2ac5d22729538f90b36c2b9d77678bb8"
entrypoint = ["java", "-jar", "{artifact}"]
health_check = "/health"
shutdown_timeout_seconds = 10

[plugin.marketplace]
title = "Published Process"
summary = "Published process test"
license = "MIT"
tags = ["test"]

[plugin.capabilities]
network = []
filesystem = []
database = false
"#
        .to_owned(),
        artifact,
        Default::default(),
    )?;
    let temporary = tempfile::tempdir()?;
    let installer = RepositoryInstaller::new(temporary.path().join("cache"));

    let published = installer.publish(&request).await?;

    assert_eq!(published.runtime, PluginRuntime::Process);
    assert!(published.pages.is_empty());
    assert!(
        installer
            .artifact(&request.rev, "dist/plugin.jar")?
            .is_file()
    );
    Ok(())
}

#[tokio::test]
async fn rejects_published_artifact_with_mismatched_digest() -> Result<()> {
    let mut request = package_test_request(b"[]")?;
    request.artifact_base64 = base64::engine::general_purpose::STANDARD.encode(b"[1]");
    let temporary = tempfile::tempdir()?;
    let installer = RepositoryInstaller::new(temporary.path().join("cache"));

    let error = match installer.publish(&request).await {
        Ok(_) => panic!("摘要错误的 artifact 不能发布"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("SHA-256"));
    Ok(())
}
