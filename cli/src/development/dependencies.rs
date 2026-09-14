use anyhow::{Context, Result, ensure};
use az_plugin_development::{
    DependencyCandidate, DevConfiguration, DevelopmentLock, LockedPlugin, PublishedRelease,
};
use az_plugin_manifest::RepositoryDependency;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(test)]
#[path = "dependencies_tests.rs"]
mod tests;

#[derive(Debug)]
pub(super) struct Workspace {
    pub root: PathBuf,
    pub source: String,
    pub config: DevConfiguration,
    pub published: bool,
}

pub(super) fn resolve(
    root: &Path,
    overrides: &[PathBuf],
    offline: bool,
) -> Result<(Vec<Workspace>, DevelopmentLock)> {
    let paths = std::iter::once(root.to_path_buf())
        .chain(overrides.iter().cloned())
        .map(|path| path.canonicalize().map_err(Into::into))
        .collect::<Result<Vec<_>>>()?;
    let declared = paths
        .iter()
        .map(|path| requirements(&std::fs::read_to_string(path.join("aio-plugin.toml"))?))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .map(|dependency| dependency.git)
        .collect::<std::collections::BTreeSet<_>>();
    let mut local = BTreeMap::new();
    for path in paths {
        let source = match git(&path, &["remote", "get-url", "origin"]) {
            Some(source) => normalize(&source)?,
            None if path != root => {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .context("依赖目录名无效")?;
                let matches = declared
                    .iter()
                    .filter(|git| git.trim_end_matches(".git").rsplit('/').next() == Some(name))
                    .collect::<Vec<_>>();
                ensure!(
                    matches.len() == 1,
                    "无远端依赖 {name} 必须与唯一声明的仓库名一致；也可通过 git remote add origin 明确来源，无需 push"
                );
                matches[0].clone()
            }
            None => az_plugin_development::source_identity(&path)?,
        };
        ensure!(
            local.insert(source, path).is_none(),
            "同一来源不能提供多个本地覆盖"
        );
    }
    let target = local
        .iter()
        .find(|(_, path)| path.as_path() == root)
        .context("目标工作区不存在")?
        .0
        .clone();
    let previous = if root.join("aio-dev.lock").is_file() {
        let lock: DevelopmentLock =
            serde_json::from_slice(&std::fs::read(root.join("aio-dev.lock"))?)?;
        ensure!(
            lock.version == 1 && lock.host_version == az_plugin_development::HOST_VERSION,
            "锁文件的宿主版本不匹配，请重新生成 aio-dev.lock"
        );
        lock.plugins
            .into_iter()
            .map(|plugin| (plugin.source.clone(), plugin))
            .collect::<BTreeMap<_, _>>()
    } else {
        BTreeMap::new()
    };
    let mut packages = super::packages::Packages::new(root, offline)?;
    let selected = az_plugin_development::resolve_dependencies(&target, |source| {
        if let Some(path) = local.get(source) {
            return Ok(vec![DependencyCandidate {
                source: source.into(),
                version: local_version(path)?,
                dependencies: requirements(&std::fs::read_to_string(
                    path.join("aio-plugin.toml"),
                )?)?,
            }]);
        }
        packages
            .candidates(source, previous.get(source))?
            .into_iter()
            .map(|release| {
                Ok(DependencyCandidate {
                    source: source.into(),
                    version: release.version.parse()?,
                    dependencies: requirements(&release.manifest)?,
                })
            })
            .collect()
    })?;
    for source in local.keys() {
        ensure!(
            selected.iter().any(|candidate| &candidate.source == source),
            "--with 指向未声明的依赖: {source}"
        );
    }
    let mut workspaces = Vec::new();
    let mut plugins = Vec::new();
    for candidate in selected {
        let release: Option<PublishedRelease> = packages
            .releases
            .get(&(candidate.source.clone(), candidate.version.to_string()))
            .cloned();
        let (path, config) = if let Some(path) = local.get(&candidate.source) {
            (path.clone(), az_plugin_development::read(path)?)
        } else {
            packages.workspace(release.as_ref().context("依赖缺少发布记录")?)?
        };
        let dependencies = candidate
            .dependencies
            .iter()
            .map(|dependency| dependency.git.clone())
            .collect::<Vec<_>>();
        let published = release.is_some();
        plugins.push(LockedPlugin {
            source: candidate.source.clone(),
            workspace: release.is_none().then(|| path.clone()),
            source_sha: release
                .as_ref()
                .map(|value| value.source_sha.clone())
                .or_else(|| {
                    git(&path, &["rev-parse", "HEAD"])
                        .filter(|sha| sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
                }),
            content_digest: az_plugin_development::fingerprint(&path, &[".".into()])?,
            package_digest: release.map(|value| value.digest),
            version: Some(candidate.version.to_string()),
            dependencies: dependencies.clone(),
        });
        workspaces.push(Workspace {
            root: path,
            source: candidate.source,
            config,
            published,
        });
    }
    Ok((
        workspaces,
        DevelopmentLock {
            version: 1,
            host_version: az_plugin_development::HOST_VERSION.into(),
            plugins,
        },
    ))
}

fn normalize(source: &str) -> Result<String> {
    let source = source
        .strip_prefix("git@")
        .and_then(|value| value.split_once(':'))
        .map(|(host, path)| format!("https://{host}/{path}"))
        .unwrap_or_else(|| source.to_owned());
    az_plugin_package::normalize_git_source(&source)
}

fn requirements(text: &str) -> Result<Vec<RepositoryDependency>> {
    let value: toml::Value = toml::from_str(text)?;
    let mut dependencies: Vec<RepositoryDependency> = value
        .get("plugin")
        .and_then(|plugin| plugin.get("dependencies"))
        .map(|items| items.clone().try_into())
        .transpose()?
        .unwrap_or_default();
    if let Some(parent) = value
        .get("plugin")
        .and_then(|plugin| plugin.get("marketplace"))
        .and_then(|marketplace| marketplace.get("parent"))
        .and_then(|parent| parent.as_str())
    {
        if !dependencies
            .iter()
            .any(|dependency| dependency.git == parent)
        {
            dependencies.push(RepositoryDependency {
                git: parent.into(),
                version: "*".into(),
            });
        }
    }
    az_plugin_manifest::validate_dependencies(&dependencies)?;
    Ok(dependencies)
}

fn local_version(path: &Path) -> Result<semver::Version> {
    for (file, section) in [("Cargo.toml", "package"), ("aio-dev.toml", "plugin")] {
        if let Ok(text) = std::fs::read_to_string(path.join(file)) {
            let value: toml::Value = toml::from_str(&text)?;
            if let Some(value) = value
                .get(section)
                .and_then(|s| s.get("version"))
                .and_then(|v| v.as_str())
            {
                return Ok(semver::Version::parse(value)?);
            }
            if let Some(version) = value
                .get("workspace")
                .and_then(|w| w.get("package"))
                .and_then(|p| p.get("version"))
                .and_then(|v| v.as_str())
            {
                return Ok(semver::Version::parse(version)?);
            }
        }
    }
    if let Ok(bytes) = std::fs::read(path.join("package.json")) {
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        if let Some(version) = value["version"].as_str() {
            return Ok(semver::Version::parse(version)?);
        }
    }
    Ok(semver::Version::new(0, 1, 0))
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let top = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !top.status.success()
        || Path::new(String::from_utf8_lossy(&top.stdout).trim())
            .canonicalize()
            .ok()?
            != root.canonicalize().ok()?
    {
        return None;
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
