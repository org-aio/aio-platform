use super::{CliConfiguration, Publication};
use crate::{CommandSpec, InstallationPlan, Requirement, ToolManifest};
use anyhow::{Result, ensure};

impl Publication {
    pub fn validate(&self) -> Result<()> {
        let name = self.package.strip_prefix('@').unwrap_or(&self.package);
        ensure!(
            !name.is_empty()
                && name.len() <= 214
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_./".contains(&b)),
            "npm 包名无效"
        );
        ensure!(
            if self.package.starts_with('@') {
                name.split('/').count() == 2
                    && name
                        .split('/')
                        .all(|s| !s.is_empty() && !s.starts_with('.'))
            } else {
                !name.contains('/') && !name.starts_with('.')
            },
            "npm 包名无效"
        );
        let version = semver::Version::parse(&self.version)?;
        ensure!(version.to_string() == self.version, "必须发布明确的 SemVer");
        Ok(())
    }
}

impl CliConfiguration {
    pub fn manifest(
        &self,
        publication: &Publication,
        git: &str,
        summary: &str,
        license: &str,
        node_version: Option<&str>,
    ) -> Result<ToolManifest> {
        publication.validate()?;
        let command = |args: Vec<String>| CommandSpec {
            program: self.command.clone(),
            args,
        };
        let npm = |args: Vec<String>| CommandSpec {
            program: "npm".into(),
            args,
        };
        let mut install = vec![npm(vec![
            "install".into(),
            "--global".into(),
            format!("{}@{}", publication.package, publication.version),
        ])];
        if !self.setup.is_empty() {
            install.push(command(self.setup.clone()));
        }
        let mut uninstall = Vec::new();
        if !self.uninstall.is_empty() {
            uninstall.push(command(self.uninstall.clone()));
        }
        uninstall.push(npm(vec![
            "uninstall".into(),
            "--global".into(),
            publication.package.clone(),
        ]));
        let plan = InstallationPlan {
            requirements: vec![Requirement {
                label: "Node.js 与 npm".into(),
                check: CommandSpec {
                    program: "node".into(),
                    args: vec!["--version".into()],
                },
                version: node_version.map(str::to_owned),
                help: "请安装符合此 CLI 要求的 Node.js 及 npm：https://nodejs.org/".into(),
            }],
            install,
            uninstall,
            detect: Some(command(vec!["--version".into()])),
        };
        let manifest = ToolManifest {
            id: self.id.clone(),
            version: publication.version.clone(),
            title: self.title.clone(),
            summary: summary.into(),
            homepage: git.into(),
            license: license.into(),
            tags: vec!["cli".into()],
            platforms: self
                .platforms
                .iter()
                .map(|p| (p.clone(), plan.clone()))
                .collect(),
        };
        manifest.validate()?;
        Ok(manifest)
    }
}
