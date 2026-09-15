use super::{Metadata, Registration};
use crate::{CommandSpec, InstallationPlan, ToolManifest};
use anyhow::{Result, ensure};

impl Metadata {
    pub fn normalize(&mut self) -> Result<()> {
        self.title = self.title.trim().into();
        self.summary = self.summary.trim().into();
        self.git = self
            .git
            .trim()
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .into();
        ensure!(
            self.title.len() <= 200 && self.summary.len() <= 4000,
            "标题或备注过长"
        );
        ensure!(self.git.len() <= 2048, "Git 地址过长");
        if !self.git.is_empty() {
            let url = url::Url::parse(&self.git)?;
            ensure!(
                url.scheme() == "https"
                    && url.host_str().is_some()
                    && url.port_or_known_default() == Some(443)
                    && url.path() != "/"
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
                    && !self.git.chars().any(char::is_whitespace),
                "请填写不含凭据的 HTTPS Git 仓库地址"
            );
        }
        Ok(())
    }
}

impl Registration {
    pub fn manifest(&self, id: String) -> Result<ToolManifest> {
        let command = self.command.trim();
        ensure!(!command.is_empty(), "请填写安装命令");
        for value in [command, &self.uninstall, &self.detect] {
            ensure!(
                value.len() <= 4096 && !value.chars().any(char::is_control),
                "请填写不超过 4096 字节的单行命令"
            );
        }
        let mut metadata = self.metadata.clone();
        metadata.normalize()?;
        if metadata.title.is_empty() {
            metadata.title = suggested_title(command, &metadata.git);
        }
        if metadata.summary.is_empty() {
            metadata.summary = format!("安装 {}", metadata.title);
        }
        let mut platforms = std::collections::BTreeMap::new();
        for platform in &self.platforms {
            ensure!(
                ["macos", "linux", "windows"].contains(&platform.as_str()),
                "不支持的平台"
            );
            platforms.insert(
                platform.clone(),
                InstallationPlan {
                    requirements: vec![],
                    install: vec![shell(platform, command)],
                    uninstall: if self.uninstall.trim().is_empty() {
                        vec![]
                    } else {
                        vec![shell(platform, self.uninstall.trim())]
                    },
                    detect: (!self.detect.trim().is_empty())
                        .then(|| shell(platform, self.detect.trim())),
                },
            );
        }
        let manifest = ToolManifest {
            id,
            version: "1.0.0".into(),
            title: metadata.title,
            summary: metadata.summary,
            homepage: metadata.git,
            license: String::new(),
            tags: vec!["cli".into()],
            platforms,
        };
        manifest.validate()?;
        Ok(manifest)
    }
}

fn shell(platform: &str, command: &str) -> CommandSpec {
    if platform == "windows" {
        CommandSpec {
            program: "powershell.exe".into(),
            args: vec![
                "-NoProfile".into(),
                "-Command".into(),
                format!(
                    "$ErrorActionPreference='Stop'; {command}; if ($LASTEXITCODE) {{ exit $LASTEXITCODE }}"
                ),
            ],
        }
    } else {
        CommandSpec {
            program: "bash".into(),
            args: vec!["-o".into(), "pipefail".into(), "-c".into(), command.into()],
        }
    }
}

pub(super) fn suggested_title(command: &str, git: &str) -> String {
    if let Some(name) = git.rsplit('/').next().filter(|s| !s.is_empty()) {
        return name.chars().take(80).collect();
    }
    let words: Vec<_> = command.split_whitespace().collect();
    let skip = match words.first().copied() {
        Some("npm" | "pnpm" | "yarn" | "pip" | "pip3" | "brew") => 2,
        Some("npx" | "uvx" | "bunx") => 1,
        _ => 0,
    };
    let name = words
        .iter()
        .skip(skip)
        .find(|v| !v.starts_with('-'))
        .copied()
        .unwrap_or("CLI 工具");
    let name = name.trim_matches(['\'', '"']);
    let name = name
        .rfind('@')
        .filter(|&at| at > 0)
        .map(|at| &name[..at])
        .unwrap_or(name);
    name.chars().take(80).collect()
}
