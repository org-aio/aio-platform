use super::super::RuntimeState;
use anyhow::{Context, Result, ensure};
use az_plugin_development::{DevArtifact, DevHostSession};
use std::path::Path;

pub(super) fn prepare(state: &RuntimeState, artifact: &DevArtifact) -> Result<std::path::PathBuf> {
    for digest in [&artifact.content_digest, &artifact.backend_digest] {
        ensure!(
            digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
            "开发产物摘要无效"
        );
    }
    let workspace = &artifact.workspace;
    let text = std::fs::read_to_string(workspace.join("aio-plugin.toml"))?;
    let manifest: toml::Value = toml::from_str(&text)?;
    let plugin = manifest.get("plugin").context("缺少插件清单")?;
    let path = |section: &str, field: &str| {
        plugin
            .get(section)
            .and_then(|value| value.get(field))
            .and_then(|value| value.as_str())
            .context("清单缺少产物路径")
    };
    let frontend = path("frontend", "path")?;
    let backend = path("runtime", "artifact")?;
    let migrations = plugin
        .get("database")
        .and_then(|value| value.get("migrations"))
        .and_then(|value| value.as_str());
    for path in [Some(frontend), Some(backend), migrations]
        .into_iter()
        .flatten()
    {
        az_plugin_development::validate_relative(path)?;
    }
    let digest = |root: &Path| {
        az_plugin_development::artifact_digest(
            &root.join("aio-plugin.toml"),
            &root.join(frontend),
            &root.join(backend),
        )
    };
    ensure!(
        az_plugin_development::artifact_digest(
            &workspace.join("aio-plugin.toml"),
            &artifact.frontend,
            &artifact.backend
        )? == artifact.content_digest,
        "构建产物已变化，拒绝激活过期内容"
    );
    ensure!(
        az_plugin_development::backend_digest(&artifact.backend)? == artifact.backend_digest,
        "后端产物摘要不匹配"
    );
    let root = state.repository.cache_root.join(&artifact.content_digest);
    let staging = state
        .repository
        .cache_root
        .join(format!(".development-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&staging)?;
    let result = (|| -> Result<()> {
        copy(&artifact.frontend, &staging.join(frontend))?;
        copy(&artifact.backend, &staging.join(backend))?;
        if let Some(path) = migrations {
            copy(&workspace.join(path), &staging.join(path))?;
        }
        std::fs::write(staging.join("aio-plugin.toml"), text.as_bytes())?;
        ensure!(
            digest(&staging)? == artifact.content_digest,
            "复制期间构建产物发生变化"
        );
        if manifest.get("schema_version").and_then(|v| v.as_integer()) == Some(2) {
            az_plugin_bundle::VerifiedBundle::from_development_directory(
                &staging,
                artifact.content_digest.clone(),
            )?;
        } else {
            az_plugin_manifest::validate_repository(&staging)?;
        }
        if root.exists() {
            ensure!(
                digest(&root)? == artifact.content_digest,
                "已缓存开发产物损坏"
            );
        } else {
            std::fs::rename(&staging, &root)?;
        }
        Ok(())
    })();
    if staging.exists() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result?;
    Ok(root)
}

pub(super) fn dependencies(
    session: &DevHostSession,
    dependencies: &[az_plugin_manifest::RepositoryDependency],
    parent: Option<&str>,
) -> Result<()> {
    let mut requirements = dependencies.to_vec();
    if let Some(parent) = parent {
        if !requirements
            .iter()
            .any(|dependency| dependency.git == parent)
        {
            requirements.push(az_plugin_manifest::RepositoryDependency {
                git: parent.into(),
                version: "*".into(),
            });
        }
    }
    for requirement in requirements {
        let locked = session
            .workspaces
            .iter()
            .find(|workspace| workspace.source == requirement.git)
            .with_context(|| {
                format!(
                    "依赖 {} 不在本次运行集合内，请重新运行 dev 并提供 --with",
                    requirement.git
                )
            })?;
        ensure!(
            az_plugin_development::matches_requirement(
                &semver::VersionReq::parse(&requirement.version)?,
                &semver::Version::parse(&locked.version)?
            ),
            "开发依赖版本约束已变化，请重新启动沙箱"
        );
    }
    Ok(())
}

fn copy(source: &Path, destination: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(source)?;
    ensure!(!meta.file_type().is_symlink(), "开发产物不能包含符号链接");
    if meta.is_dir() {
        std::fs::create_dir_all(destination)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            copy(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else {
        ensure!(meta.is_file(), "开发产物只能是普通文件");
        std::fs::create_dir_all(destination.parent().context("产物路径无效")?)?;
        std::fs::copy(source, destination)?;
    }
    Ok(())
}
