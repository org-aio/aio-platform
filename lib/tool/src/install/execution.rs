use crate::{CommandSpec, Requirement};
use anyhow::{Context as _, Result, ensure};
use std::process::{Command, Stdio};

fn command(spec: &CommandSpec) -> Result<Command> {
    #[cfg(windows)]
    let program = {
        let found = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .flat_map(|dir| {
                if std::path::Path::new(&spec.program).extension().is_some() {
                    vec![dir.join(&spec.program)]
                } else {
                    ["exe", "cmd", "bat"]
                        .map(|extension| dir.join(format!("{}.{extension}", spec.program)))
                        .to_vec()
                }
            })
            .find(|path| path.is_file());
        if let Some(path) = &found
            && path.extension().is_some_and(|ext| {
                ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat")
            })
        {
            ensure!(
                spec.args
                    .iter()
                    .all(|arg| !arg.chars().any(|c| "&|<>^%\"!\r\n".contains(c))),
                "批处理参数含不支持的 shell 字符"
            );
        }
        found.unwrap_or_else(|| spec.program.clone().into())
    };
    #[cfg(not(windows))]
    let program = &spec.program;
    let mut command = Command::new(program);
    command.args(&spec.args).stdin(Stdio::inherit());
    Ok(command)
}

pub(super) fn run(spec: &CommandSpec) -> Result<()> {
    println!(
        "执行：{} {}",
        spec.program,
        serde_json::to_string(&spec.args)?
    );
    let status = command(spec)?
        .status()
        .with_context(|| format!("无法运行 {}，请检查 PATH 和安装依赖", spec.program))?;
    ensure!(status.success(), "{} 返回失败状态 {status}", spec.program);
    Ok(())
}

pub(super) fn requirements(requirements: &[Requirement]) -> Result<()> {
    for requirement in requirements {
        let check = || -> Result<()> {
            let output = command(&requirement.check)?.output()?;
            ensure!(output.status.success(), "依赖检测失败");
            if let Some(version) = &requirement.version {
                let text = String::from_utf8_lossy(&output.stdout);
                let value = text.trim().trim_start_matches('v');
                ensure!(
                    semver::VersionReq::parse(version)?.matches(&semver::Version::parse(value)?),
                    "需要版本 {version}，当前为 {value}"
                );
            }
            Ok(())
        };
        check().with_context(|| format!("缺少依赖 {}。{}", requirement.label, requirement.help))?;
    }
    Ok(())
}
