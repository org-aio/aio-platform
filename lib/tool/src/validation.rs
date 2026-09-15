use crate::{CommandSpec, ToolManifest};
use anyhow::{Result, ensure};

pub(crate) fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl ToolManifest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            identifier(&self.id),
            "工具 ID 只能使用小写字母、数字和连字符"
        );
        let version = semver::Version::parse(&self.version)?;
        ensure!(
            version.to_string() == self.version,
            "版本必须是明确的 SemVer"
        );
        ensure!(
            !self.title.trim().is_empty() && self.title.len() <= 200 && self.summary.len() <= 4000,
            "名称不能为空，名称与备注不能超过长度限制"
        );
        if !self.homepage.is_empty() {
            let homepage = url::Url::parse(&self.homepage)?;
            ensure!(
                homepage.scheme() == "https"
                    && homepage.host_str().is_some()
                    && homepage.username().is_empty()
                    && homepage.password().is_none(),
                "主页必须是 HTTPS 地址"
            );
        }
        ensure!(
            self.tags.iter().any(|tag| tag == "cli"),
            "工具条目必须包含 cli 标签"
        );
        ensure!(!self.platforms.is_empty(), "必须声明支持的平台");
        for (platform, plan) in &self.platforms {
            ensure!(
                ["macos", "linux", "windows"].contains(&platform.as_str()),
                "不支持的平台: {platform}"
            );
            ensure!(!plan.install.is_empty(), "必须声明安装步骤");
            ensure!(
                plan.install.len() <= 20
                    && plan.uninstall.len() <= 20
                    && plan.requirements.len() <= 20,
                "安装步骤过多"
            );
            for command in plan
                .install
                .iter()
                .chain(&plan.uninstall)
                .chain(plan.detect.iter())
                .chain(plan.requirements.iter().map(|r| &r.check))
            {
                command.validate()?;
            }
            for requirement in &plan.requirements {
                if let Some(version) = &requirement.version {
                    semver::VersionReq::parse(version)?;
                }
            }
        }
        Ok(())
    }
}

impl CommandSpec {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.program.is_empty()
                && self.program.len() <= 128
                && self
                    .program
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)),
            "程序必须是可执行文件名称"
        );
        ensure!(
            self.args.len() <= 64
                && self
                    .args
                    .iter()
                    .all(|arg| arg.len() <= 4096 && !arg.chars().any(char::is_control)),
            "命令参数无效"
        );
        Ok(())
    }
}
