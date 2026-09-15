use super::{identity::Identity, model::NpmPackage};
use anyhow::{Result, ensure};
use az_tool::{ToolManifest, publication::Publication};

pub(super) fn validate(
    request: &Publication,
    identity: &Identity,
    package: &NpmPackage,
) -> Result<ToolManifest> {
    ensure!(
        package.name == request.package && package.version == request.version,
        "npm 包或版本不匹配"
    );
    let source = &package.aio.source;
    ensure!(
        source.repository == identity.repository
            && source.revision == identity.sha
            && source.reference == identity.reference,
        "npm 包来源与签名发布身份不匹配"
    );
    let repository = package
        .repository
        .as_str()
        .or_else(|| package.repository.get("url").and_then(|v| v.as_str()))
        .unwrap_or_default()
        .trim_start_matches("git+")
        .trim_end_matches(".git");
    ensure!(
        repository == format!("https://github.com/{}", identity.repository),
        "npm 仓库不匹配"
    );
    ensure!(
        package.dist.integrity.starts_with("sha512-") && package.dist.integrity.len() == 95,
        "npm 包缺少 SHA-512 完整性记录"
    );
    let cli = &package.aio.cli;
    ensure!(
        package
            .bin
            .get(&cli.command)
            .and_then(|v| v.as_str())
            .is_some()
            || (package.name == cli.command && package.bin.is_string()),
        "npm 包未声明指定 CLI 命令"
    );
    cli.manifest(
        request,
        &format!("https://github.com/{}", identity.repository),
        &package.description,
        &package.license,
        package.engines.get("node").map(String::as_str),
    )
}
