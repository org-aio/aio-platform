use crate::RepositoryDependency;
use anyhow::{Result, ensure};

pub fn validate_dependencies(dependencies: &[RepositoryDependency]) -> Result<()> {
    ensure!(dependencies.len() <= 64, "插件依赖数量超过限制");
    let mut sources = std::collections::BTreeSet::new();
    for dependency in dependencies {
        let source = url::Url::parse(&dependency.git)?;
        ensure!(
            source.scheme() == "https"
                && source.host_str().is_some()
                && source.username().is_empty()
                && source.password().is_none()
                && source.query().is_none()
                && source.fragment().is_none()
                && source.path().ends_with(".git")
                && !source.path().contains("//"),
            "依赖来源必须是无凭据的 HTTPS Git 仓库"
        );
        ensure!(
            sources.insert(source.to_string()),
            "重复声明插件依赖: {}",
            dependency.git
        );
        semver::VersionReq::parse(&dependency.version)?;
    }
    Ok(())
}
